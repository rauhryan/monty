//! Generator-expression semantics and independent checkpoint continuations.

use monty::{Dump, MontyRepl, MontyRun, RunProgress, Session, SessionRef, dump};
use monty_types::{
    CompileOptions, ExcType, MontyObject, PrintWriter, ResourceTracker,
    unstable::{self, MontyNode},
};

/// The Python fixture also runs under CPython in the differential case harness.
#[test]
fn expression_semantics() {
    let mut run = MontyRun::new(
        include_str!("../test_cases/generator__expression.py").to_owned(),
        "genexpr.py",
        vec![],
        CompileOptions::default(),
    )
    .unwrap();
    run.run(vec![], ResourceTracker::default(), PrintWriter::Disabled)
        .unwrap();
}

/// Restoring twice must neither repeat filters nor share iterator progress.
#[test]
fn expression_snapshot() {
    let mut repl = MontyRepl::new("genexpr.py", ResourceTracker::default(), CompileOptions::default());
    repl.feed_run(
        "events = []\ndef keep(x):\n    events.append(x)\n    return x % 2 == 0\ng = (x for x in range(8) if keep(x))\nassert next(g) == 0",
        vec![],
        PrintWriter::Disabled,
    )
    .unwrap();
    let bytes = dump("genexpr.py", None, &[], SessionRef::Idle(&repl)).unwrap();
    for _ in 0..2 {
        let Session::Idle(mut restored) = Dump::load(&bytes).unwrap().state else {
            panic!("expected idle session");
        };
        restored
            .feed_run(
                "assert events == [0]\nassert next(g) == 2\nassert events == [0, 1, 2]\nassert list(g) == [4, 6]\nassert events == list(range(8))",
                vec![],
                PrintWriter::Disabled,
            )
            .unwrap();
    }
    repl.feed_run("assert next(g) == 2", vec![], PrintWriter::Disabled)
        .unwrap();
}

/// Host calls inside a lazy fold keep their exact position across checkpointing.
#[test]
fn expression_host_call_snapshot() {
    let runner = MontyRun::new(
        "sum(ext_fn(x) for x in [1, 2, 3])".to_owned(),
        "genexpr.py",
        vec!["ext_fn".to_owned()],
        CompileOptions::default(),
    )
    .unwrap();
    let progress = runner
        .start(
            vec![MontyObject::function("ext_fn", None)],
            ResourceTracker::default(),
            PrintWriter::Disabled,
        )
        .unwrap();
    let RunProgress::FunctionCall(call) = progress else {
        panic!("expected first callback")
    };
    assert_eq!(call.args.args().collect::<Vec<_>>(), vec![MontyObject::int(1)]);
    let progress = call.resume(MontyObject::int(10), PrintWriter::Disabled).unwrap();
    let bytes = dump("genexpr.py", None, &[], SessionRef::Running(&progress)).unwrap();
    for _ in 0..2 {
        let Session::Running(restored) = Dump::load(&bytes).unwrap().state else {
            panic!("expected running session")
        };
        complete_fold(*restored);
    }
    complete_fold(progress);
}

/// Only the remaining callbacks execute after restoring the in-flight fold.
fn complete_fold(mut progress: RunProgress) {
    for value in [2, 3] {
        let RunProgress::FunctionCall(call) = progress else {
            panic!("expected callback")
        };
        assert_eq!(call.args.args().collect::<Vec<_>>(), vec![MontyObject::int(value)]);
        progress = call
            .resume(MontyObject::int(value * 10), PrintWriter::Disabled)
            .unwrap();
    }
    assert_eq!(progress.into_complete().unwrap(), MontyObject::int(60));
}

/// Native consumers retain their existing no-host-suspension boundary and unwind cleanly.
#[test]
fn native_consumer_suspension_boundary() {
    let mut runner = MontyRun::new(
        r"
import math
seen = []
def source():
    try:
        yield ext_fn()
    finally:
        seen.append('closed')
g = source()
try:
    math.fsum(g)
except NotImplementedError:
    pass
else:
    assert False
assert seen == ['closed']
assert next(g, None) is None
assert math.fsum(x for x in [1, 2]) == 3
"
        .to_owned(),
        "genexpr.py",
        vec!["ext_fn".to_owned()],
        CompileOptions::default(),
    )
    .unwrap();
    runner
        .run_no_limits(vec![MontyObject::function("ext_fn", None)])
        .unwrap();
}

/// Unsupported outputs preserve type independently of their repr payload.
#[test]
fn output_type_metadata() {
    let mut runner = MontyRun::new(
        "[{}.keys(), {}.values(), {}.items(), (x for x in [1]), iter([1])]".to_owned(),
        "types.py",
        vec![],
        CompileOptions::default(),
    )
    .unwrap();
    let output = runner.run_no_limits(vec![]).unwrap();
    let items = output.as_ref().items().unwrap();
    for (item, expected) in items
        .iter()
        .zip(["dict_keys", "dict_values", "dict_items", "generator", "list_iterator"])
    {
        assert_eq!(item.type_name(), expected);
        assert!(matches!(unstable::node(*item), MontyNode::TypedRepr { .. }));
    }
}

/// Flat syntax cannot bypass the depth bound of its lowered nested statements.
#[test]
fn expression_synthetic_depth_is_bounded() {
    for tail in [
        " for x in [1]".repeat(1000),
        format!(" for x in [1]{}", " if True".repeat(1000)),
    ] {
        let error = MontyRun::new(format!("(x{tail})"), "depth.py", vec![], CompileOptions::default()).unwrap_err();
        assert_eq!(error.exc_type(), ExcType::SyntaxError);
    }
}
