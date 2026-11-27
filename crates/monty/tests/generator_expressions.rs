//! Generator-expression semantics and independent checkpoint continuations.

use monty::{Dump, MontyRepl, MontyRun, RunProgress, Session, SessionRef, dump};
use monty_types::{CompileOptions, ExcType, MontyObject, PrintWriter, ResourceTracker};

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

/// Unpacking and collection keep the same host-call boundary for every iterable shape.
#[test]
fn suspendable_consumers_checkpoint_each_item() {
    for (body, count) in [
        ("assert [*source] == [1, 2, 3]", 3),
        ("assert (*source,) == (1, 2, 3)", 3),
        ("assert {*source} == {1, 2, 3}", 3),
        ("def take(a, b, c):\n    return a + b + c\nassert take(*source) == 6", 3),
        ("a, b, c = source\nassert (a, b, c) == (1, 2, 3)", 3),
        ("a, *b = source\nassert (a, b) == (1, [2, 3])", 3),
        ("assert list(source) == [1, 2, 3]", 3),
        ("assert tuple(source) == (1, 2, 3)", 3),
        ("assert sorted(source, reverse=True) == [3, 2, 1]", 3),
        ("assert ''.join(str(x) for x in source) == '123'", 3),
        ("assert sum(source) == 6", 3),
        ("assert min(source) == 1", 3),
        ("assert max(source) == 3", 3),
        ("assert all(source)", 3),
        ("assert any(source)", 1),
    ] {
        for factory in [
            "source = (ext_fn(x) for x in [1, 2, 3])",
            "class Source:\n    def __iter__(self):\n        return (ext_fn(x) for x in [1, 2, 3])\nsource = Source()",
            "class Source:\n    def __init__(self):\n        self.i = 0\n    def __iter__(self):\n        return self\n    def __next__(self):\n        self.i += 1\n        if self.i > 3:\n            raise StopIteration\n        return ext_fn(self.i)\nsource = Source()",
        ] {
            let source = format!("{factory}\n{body}");
            let runner = MontyRun::new(
                source.clone(),
                "consumers.py",
                vec!["ext_fn".to_owned()],
                CompileOptions::default(),
            )
            .unwrap();
            let mut progress = runner
                .start(
                    vec![MontyObject::function("ext_fn", None)],
                    ResourceTracker::default(),
                    PrintWriter::Disabled,
                )
                .unwrap();
            for expected in 1..=count {
                let bytes = dump("consumers.py", None, &[], SessionRef::Running(&progress)).unwrap();
                let Session::Running(restored) = Dump::load(&bytes).unwrap().state else {
                    panic!("running session")
                };
                // Resume an independent snapshot; aborting the prior state must release every owner.
                progress
                    .into_function_call()
                    .unwrap()
                    .abort(
                        monty_types::MontyException::new(ExcType::RuntimeError, Some("abandon original".to_owned())),
                        PrintWriter::Disabled,
                    )
                    .unwrap_err();
                progress = *restored;
                let RunProgress::FunctionCall(call) = progress else {
                    panic!("missing call {expected}: {source}")
                };
                let value = call.args.args().next().unwrap().to_owned();
                assert_eq!(value, MontyObject::int(expected), "{source}");
                progress = call
                    .resume(value, PrintWriter::Disabled)
                    .unwrap_or_else(|error| panic!("{source}: {error}"));
            }
            progress
                .into_complete()
                .unwrap_or_else(|| panic!("not complete: {source}"));
        }
    }
}

/// Fixed-length assignment pulls only enough items to detect an excess.
#[test]
fn bounded_unpacking_does_not_exhaust_the_source() {
    let mut runner = MontyRun::new(
        r"
seen = []
def source():
    i = 0
    while True:
        seen.append(i)
        yield i
        i += 1
g = source()
try:
    a, b = g
except ValueError:
    pass
else:
    assert False
assert seen == [0, 1, 2]
assert next(g) == 3
"
        .to_owned(),
        "bounded.py",
        vec![],
        CompileOptions::default(),
    )
    .unwrap();
    runner.run_no_limits(vec![]).unwrap();
}

/// A key callback follows the same frame path for a list and a generator.
#[test]
fn extremum_keys_suspend_for_both_call_forms() {
    for expression in ["min([1, 2, 3], key=ext_fn)", "max(1, 2, 3, key=ext_fn)"] {
        let runner = MontyRun::new(
            expression.to_owned(),
            "keys.py",
            vec!["ext_fn".to_owned()],
            CompileOptions::default(),
        )
        .unwrap();
        let mut progress = runner
            .start(
                vec![MontyObject::function("ext_fn", None)],
                ResourceTracker::default(),
                PrintWriter::Disabled,
            )
            .unwrap();
        for expected in 1..=3 {
            let RunProgress::FunctionCall(call) = progress else {
                panic!("key callback")
            };
            assert_eq!(call.args.args().next().unwrap().to_owned(), MontyObject::int(expected));
            progress = call.resume(MontyObject::int(-expected), PrintWriter::Disabled).unwrap();
        }
        assert_eq!(
            progress.into_complete().unwrap(),
            MontyObject::int(if expression.starts_with("min") { 3 } else { 1 })
        );
    }
}

/// Walrus assignment honors an enclosing global declaration.
#[test]
fn expression_walrus_global_binding() {
    let mut runner = MontyRun::new(
        "y = 0\ndef f():\n    global y\n    return list((y := x) for x in [1, 2])\nassert f() == [1, 2]\nassert y == 2"
            .to_owned(),
        "scope.py",
        vec![],
        CompileOptions::default(),
    )
    .unwrap();
    runner.run_no_limits(vec![]).unwrap();
}

/// Invalid assignment expressions must be rejected before they can bind names.
#[test]
fn expression_walrus_restrictions() {
    for (source, expected) in [
        ("(x for x in (saved := [1]))", ExcType::SyntaxError),
        ("(x for x in [1] for y in (saved := [x]))", ExcType::SyntaxError),
        ("((x := 1) for x in [1])", ExcType::SyntaxError),
        ("(x for x in (lambda: (saved := [1]))())", ExcType::SyntaxError),
        // Monty rejects all class-body walrus bindings before preparing the expression.
        (
            "class C:\n    g = ((y := x) for x in [1])",
            ExcType::NotImplementedError,
        ),
    ] {
        let error = MontyRun::new(source.to_owned(), "scope.py", vec![], CompileOptions::default()).unwrap_err();
        assert_eq!(error.exc_type(), expected, "{source}");
    }
}

/// Consumers acquire a custom iterator exactly once, even when it is its own iterator.
#[test]
fn consumer_iterator_acquisition_is_once() {
    for expression in [
        "all(source)",
        "any(source)",
        "sum(source)",
        "min(source)",
        "max(source)",
        "sorted(source)",
        "list(source)",
        "tuple(source)",
        "[*source]",
        "''.join(str(x) for x in source)",
    ] {
        let code = format!(
            r"
class Source:
    def __init__(self):
        self.acquired = 0
        self.i = 0
    def __iter__(self):
        self.acquired += 1
        assert self.acquired == 1
        return self
    def __next__(self):
        self.i += 1
        if self.i > 2:
            raise StopIteration
        return self.i
source = Source()
{expression}
assert source.acquired == 1
"
        );
        let mut runner = MontyRun::new(code, "once.py", vec![], CompileOptions::default()).unwrap();
        runner.run_no_limits(vec![]).unwrap();
    }
}
