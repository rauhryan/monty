//! Native1 transport: one real Scheduler future per cut. No guest scheduling here.
use std::{env, fs};

use monty::{Dump, MontyRepl, ReplProgress, Session, SessionRef, dump};
use monty_types::{
    CompileOptions, ExcType, ExtFunctionResult, MontyException, MontyObject, PrintWriter, ResourceTracker,
};
use serde_json::{Value, json};

/// Settle the one owned native future; errors are injected by Rust, not guest throw.
fn resume(progress: ReplProgress, text: &str) -> ReplProgress {
    let pending = progress.into_resolve_futures().expect("native Scheduler cut");
    assert_eq!(pending.pending_call_ids().len(), 1);
    let id = pending.pending_call_ids()[0];
    let reply: Value = serde_json::from_str(text).unwrap();
    let cancel = reply.get("native_cancel").map(|r| r.as_str().unwrap().to_owned());
    if cancel.is_some() || reply.get("native_results").is_some() {
        let results = reply
            .get("native_results")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|result| {
                let call_id = result
                    .get("id")
                    .map_or(id, |v| u32::try_from(v.as_u64().unwrap()).expect("call id fits u32"));
                let answer = &result["reply"];
                let outcome = if let Some(error) = answer.get("raise") {
                    ExtFunctionResult::Error(MontyException::new(
                        ExcType::ValueError,
                        Some(error.as_str().unwrap().into()),
                    ))
                } else {
                    ExtFunctionResult::Return(MontyObject::string(answer.to_string()))
                };
                (call_id, outcome)
            })
            .collect();
        return pending.resume_native(results, cancel, PrintWriter::Stdout).unwrap();
    }
    let outcome = if let Some(error) = reply.get("raise") {
        ExtFunctionResult::Error(MontyException::new(
            ExcType::ValueError,
            Some(error.as_str().unwrap().into()),
        ))
    } else {
        ExtFunctionResult::Return(MontyObject::string(text))
    };
    pending.resume(vec![(id, outcome)], PrintWriter::Stdout).unwrap()
}

/// Acknowledge a real host call as pending before exposing the quiescent cut.
fn boundary(progress: ReplProgress) -> (ReplProgress, Value) {
    match progress {
        ReplProgress::FunctionCall(call) => {
            assert_eq!(call.function_name, "exchange");
            assert_eq!(call.args.args().len(), 1);
            assert_eq!(call.args.kwargs().len(), 0);
            let view: Value = serde_json::from_str(call.args.arg(0).unwrap().as_str().unwrap()).unwrap();
            let pending = call.resume_pending(PrintWriter::Stdout).unwrap();
            let ReplProgress::ResolveFutures(ref state) = pending else {
                panic!("pending");
            };
            let ids = state.pending_call_ids().to_vec();
            (pending, json!({"effect": view, "raw_pending_ids": ids}))
        }
        ReplProgress::Complete { ref value, .. } => {
            let view = if let Some(text) = value.as_ref().as_str() {
                serde_json::from_str(text).unwrap()
            } else {
                decode_cancel(value)
            };
            let raw = serde_json::to_value(value).expect("raw envelope receipt");
            (progress, json!({"complete": view, "raw_envelope": raw}))
        }
        ReplProgress::ResolveFutures(ref state) => {
            let ids = state.pending_call_ids().to_vec();
            (progress, json!({"waiting":true, "raw_pending_ids": ids}))
        }
        other => panic!("unexpected boundary {other:?}"),
    }
}

/// Decode exactly the bounded cancellation schema from checked raw pairs.
fn decode_cancel(value: &MontyObject) -> Value {
    use monty_types::unstable::{MontyGraph, MontyNode, NodeId, graph_parts};
    fn record(graph: &MontyGraph, id: NodeId, summary: bool) -> Value {
        let MontyNode::Dict(pairs) = graph.nodes().get(id.index()).expect("record reference") else {
            panic!("expected dict");
        };
        let keys: &[&str] = if summary {
            &["type", "message", "message_truncated", "representation"]
        } else {
            &[
                "status",
                "reason",
                "value",
                "error",
                "interrupted_error",
                "value_export_error",
            ]
        };
        let mut seen = std::collections::HashSet::new();
        for (key, value) in pairs {
            let Some(MontyNode::String(key)) = graph.nodes().get(key.index()) else {
                panic!("text key required");
            };
            assert!(
                keys.contains(&key.as_str()) && seen.insert(key.as_str()),
                "unknown or duplicate key"
            );
            assert!(graph.nodes().get(value.index()).is_some(), "value reference");
        }
        assert_eq!(seen.len(), keys.len(), "missing key");
        let mut output = serde_json::Map::new();
        for (key, value) in pairs {
            let MontyNode::String(key) = graph.node(*key) else {
                unreachable!()
            };
            let node = graph.node(*value);
            let decoded = if !summary && key == "interrupted_error" && !matches!(node, MontyNode::None) {
                record(graph, *value, true)
            } else if summary && key == "message_truncated" {
                let MontyNode::Bool(b) = node else {
                    panic!("boolean required");
                };
                Value::Bool(*b)
            } else {
                match node {
                    MontyNode::String(text) => Value::String(text.clone()),
                    MontyNode::None
                        if matches!(
                            key.as_str(),
                            "message" | "value" | "error" | "interrupted_error" | "value_export_error"
                        ) =>
                    {
                        Value::Null
                    }
                    _ => panic!("text or explicit nullable field required"),
                }
            };
            output.insert(key.clone(), decoded);
        }
        Value::Object(output)
    }
    let (graph, root) = graph_parts(value);
    MontyGraph::validate(graph.nodes()).expect("valid graph references");
    record(graph, root, false)
}

/// Separate processes own producers, consumers and physical disposers.
fn main() {
    let args: Vec<String> = env::args().collect();
    let progress = if args[1] == "start" || args[1] == "run" {
        MontyRepl::new("native1.py", ResourceTracker::default(), CompileOptions::default())
            .feed_start(
                &fs::read_to_string(&args[2]).unwrap(),
                vec![("exchange".into(), MontyObject::function("exchange", None))],
                PrintWriter::Stdout,
            )
            .unwrap()
    } else {
        let Session::Suspended(progress) = Dump::load(&fs::read(&args[2]).unwrap()).unwrap().state else {
            panic!("suspended")
        };
        assert!(matches!(*progress, ReplProgress::ResolveFutures(_)));
        if args[1] == "dispose" {
            drop(progress.into_repl());
            println!("{{\"disposed\":true}}");
            return;
        }
        resume(*progress, &args[3])
    };
    let (mut progress, mut view) = boundary(progress);
    if args[1] == "run" {
        let batches: Vec<Value> = serde_json::from_str(&fs::read_to_string(&args[3]).unwrap()).unwrap();
        let mut raw = vec![view.clone()];
        let mut views = vec![view["effect"].clone()];
        for batch in batches {
            (progress, view) = boundary(resume(progress, &batch.to_string()));
            raw.push(view.clone());
            views.push(
                view.get("effect")
                    .or_else(|| view.get("complete"))
                    .unwrap_or_else(|| views.last().unwrap())
                    .clone(),
            );
        }
        assert!(matches!(progress, ReplProgress::Complete { .. }));
        println!("{}", json!({"views": views, "raw": raw}));
    } else {
        if matches!(progress, ReplProgress::ResolveFutures(_)) {
            fs::write(
                args.last().unwrap(),
                dump("native1.py", None, &[], SessionRef::Suspended(&progress)).unwrap(),
            )
            .unwrap();
        }
        println!("{view}");
    }
    // Original suspended frames/futures are physically disposed without guest cleanup.
    drop(progress.into_repl());
}
