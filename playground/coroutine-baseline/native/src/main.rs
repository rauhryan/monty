//! One process per advance; only trusted snapshot bytes cross process boundaries.
use std::{env, fs};

use monty::{Dump, MontyRepl, ReplProgress, Session, SessionRef, dump};
use monty_types::{
    CallArgs, CompileOptions, ExcType, ExtFunctionResult, MontyException, MontyObject, PrintWriter, ResourceTracker,
};
use serde_json::{Value, json};

/// Validate the complete saved or fresh call before any boundary action.
fn validate_exchange(name: &str, args: &CallArgs) {
    assert!(
        name == "exchange" && args.args().len() == 1 && args.kwargs().len() == 0,
        "invalid exchange arguments"
    );
    assert!(args.arg(0).unwrap().as_str().is_some(), "expected guest JSON text");
}

/// Execute one bounded host turn, explicitly dropping original and restored states.
fn main() {
    let args: Vec<String> = env::args().collect();
    let result = if args[1] == "start" {
        MontyRepl::new("baseline.py", ResourceTracker::default(), CompileOptions::default()).feed_start(
            &fs::read_to_string(&args[2]).unwrap(),
            vec![(
                "exchange".to_owned(),
                MontyObject::function("exchange".to_owned(), None),
            )],
            PrintWriter::Stdout,
        )
    } else {
        let Session::Suspended(progress) = Dump::load(&fs::read(&args[2]).unwrap()).unwrap().state else {
            panic!("expected suspended repl")
        };
        let ReplProgress::FunctionCall(call) = &*progress else {
            panic!("expected host call")
        };
        validate_exchange(&call.function_name, &call.args);
        if args[1] == "dispose" {
            drop(progress);
            println!("{{\"disposed\":true}}");
            return;
        }
        let ReplProgress::FunctionCall(call) = *progress else {
            panic!("expected host call")
        };
        let reply: Value = serde_json::from_str(&args[3]).unwrap();
        let answer = if let Some(message) = reply.get("raise") {
            ExtFunctionResult::Error(MontyException::new(
                ExcType::ValueError,
                Some(message.as_str().unwrap().to_owned()),
            ))
        } else {
            ExtFunctionResult::Return(MontyObject::string(&args[3]))
        };
        call.resume(answer, PrintWriter::Stdout)
    };
    match result {
        Ok(progress) => match &progress {
            ReplProgress::FunctionCall(call) => {
                validate_exchange(&call.function_name, &call.args);
                let arg = call.args.arg(0).unwrap();
                let effect = arg.as_str().unwrap();
                let path = args.last().unwrap();
                fs::write(
                    path,
                    dump("baseline.py", None, &[], SessionRef::Suspended(&progress)).unwrap(),
                )
                .unwrap();
                // Preserve guest number tokens and duplicate keys for the strict host decoder.
                println!("{{\"effect\":{effect}}}");
                drop(progress);
            }
            ReplProgress::Complete { value, .. } => {
                let terminal = value.as_ref();
                println!("{{\"complete\":{}}}", terminal.as_str().unwrap());
            }
            other => panic!("unexpected suspension: {other:?}"),
        },
        Err(error) => {
            println!(
                "{}",
                json!({"error":{"type":error.error.exc_type().to_string(),"message":error.error.message(),"diagnostic":error.error.to_string()}})
            );
            std::process::exit(1);
        }
    }
}
