//! Public async acceptance transport: no guest prelude or source rewriting.
use std::{env, fs};

use monty::{Dump, MontyRepl, ReplProgress, Session, SessionRef, dump};
use monty_types::{
    CompileOptions, ExtFunctionResult, MontyException, MontyObject, PrintWriter, ResourceTracker,
    unstable::{MontyGraph, MontyNode, NodeId, graph_parts},
};
use serde_json::{Value, json};

fn start(source: &str) -> ReplProgress {
    let mut repl = MontyRepl::new(
        "concurrent_io.py",
        ResourceTracker::default(),
        CompileOptions::default(),
    );
    repl.feed_start(
        &fs::read_to_string(source).unwrap(),
        vec![("exchange".into(), MontyObject::function("exchange", None))],
        PrintWriter::Stdout,
    )
    .unwrap()
}

fn resume(progress: ReplProgress, batch: &Value) -> ReplProgress {
    if batch.get("ack").is_some() {
        let ReplProgress::FunctionCall(call) = progress else {
            panic!("call acknowledgement")
        };
        return call.resume_pending(PrintWriter::Stdout).unwrap();
    }
    let cancel = batch.get("cancel").map(|value| value.as_str().unwrap().to_owned());
    let results = batch
        .get("results")
        .unwrap_or(batch)
        .as_array()
        .unwrap()
        .iter()
        .map(|reply| {
            (
                u32::try_from(reply["id"].as_u64().unwrap()).unwrap(),
                ExtFunctionResult::Return(MontyObject::string(reply["value"].as_str().unwrap())),
            )
        })
        .collect();
    progress
        .into_resolve_futures()
        .unwrap()
        .resume_native(results, cancel, PrintWriter::Stdout)
        .unwrap()
}

fn boundary(mut progress: ReplProgress, step: bool) -> (ReplProgress, Value) {
    if step && let ReplProgress::FunctionCall(call) = &progress {
        let view = json!({"call":{"id":call.call_id,"label":call.args.arg(0).unwrap().as_str().unwrap()}});
        return (progress, view);
    }
    let mut calls = Vec::new();
    while let ReplProgress::FunctionCall(call) = progress {
        assert_eq!(call.function_name, "exchange");
        calls.push(json!({"id": call.call_id, "label": call.args.arg(0).unwrap().as_str().unwrap()}));
        progress = call.resume_pending(PrintWriter::Stdout).unwrap();
    }
    let view = match &progress {
        ReplProgress::ResolveFutures(pending) => {
            let mut ids = pending.pending_call_ids().to_vec();
            ids.sort_unstable();
            json!({"calls": calls, "pending": ids})
        }
        ReplProgress::Complete { value, .. } => {
            json!({"complete": if let Some(text) = value.as_ref().as_str() {
                serde_json::from_str::<Value>(text).unwrap()
            } else { decode_cancel(value) }})
        }
        other => panic!("unexpected {other:?}"),
    };
    (progress, view)
}

/// Decode exactly the bounded cancellation schema from checked raw pairs.
fn decode_cancel(value: &MontyObject) -> Value {
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

fn main() {
    let args: Vec<String> = env::args().collect();
    let step = args[1].ends_with("-step");
    let mode = args[1].trim_end_matches("-step");
    let progress = if mode == "start" || mode == "run" {
        start(&args[2])
    } else {
        let Session::Suspended(progress) = Dump::load(&fs::read(&args[2]).unwrap()).unwrap().state else {
            panic!("suspended")
        };
        if mode == "dispose" {
            drop(progress.into_repl());
            println!("{{\"disposed\":true}}");
            return;
        }
        if mode == "reject" {
            let ReplProgress::ResolveFutures(pending) = progress.as_ref() else {
                panic!("pending")
            };
            let batch: Value = serde_json::from_str(&args[3]).unwrap();
            let results: Result<Vec<_>, _> = batch
                .as_array()
                .unwrap()
                .iter()
                .map(|reply| {
                    let id = reply["id"]
                        .as_u64()
                        .and_then(|id| u32::try_from(id).ok())
                        .ok_or_else(|| MontyException::runtime_error("invalid call_id: expected u32"))?;
                    Ok((
                        id,
                        ExtFunctionResult::Return(MontyObject::string(reply["value"].as_str().unwrap())),
                    ))
                })
                .collect();
            let error = results
                .and_then(|results| pending.validate_results(&results))
                .expect_err("invalid batch rejected by Rust");
            fs::write(
                args.last().unwrap(),
                dump("concurrent_io.py", None, &[], SessionRef::Suspended(&progress)).unwrap(),
            )
            .unwrap();
            println!("{}", json!({"rejected":error.to_string()}));
            drop(progress.into_repl());
            return;
        }
        resume(*progress, &serde_json::from_str(&args[3]).unwrap())
    };
    let (mut progress, mut view) = boundary(progress, step);
    if mode == "run" {
        let batches: Vec<Value> = serde_json::from_str(&fs::read_to_string(&args[3]).unwrap()).unwrap();
        let mut views = vec![view];
        for batch in batches {
            (progress, view) = boundary(resume(progress, &batch), step);
            views.push(view);
        }
        assert!(matches!(progress, ReplProgress::Complete { .. }));
        println!("{}", json!(views));
    } else {
        if matches!(
            progress,
            ReplProgress::ResolveFutures(_) | ReplProgress::FunctionCall(_)
        ) {
            fs::write(
                args.last().unwrap(),
                dump("concurrent_io.py", None, &[], SessionRef::Suspended(&progress)).unwrap(),
            )
            .unwrap();
        }
        println!("{view}");
    }
    drop(progress.into_repl());
}
