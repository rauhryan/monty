//! Focused generator ownership checks run before interpreter/heap teardown.
#![cfg(feature = "ref-count-return")]

use std::collections::HashMap;

use monty::{Dump, MontyRepl, MontyRun, ReplProgress, Session, SessionRef, dump};
use monty_types::{
    CompileOptions, ExcType, ExtFunctionResult, MontyException, MontyObject, MontyUuid, PrintWriter, ResourceLimits,
    ResourceTracker,
};

/// Lazy binding must retain each alias once without entering the function body.
#[test]
fn lazy_binding_and_default_aliases() {
    let output = checked_run(
        r"
events = []
token = []
def factory(captured):
    def leaf(a, b=token, *args, flag=token, **kwargs):
        events.append('entered')
        assert a is b and b is flag and flag is captured
        assert args == (token,) and kwargs['extra'] is token
        yield captured
    return leaf
leaf = factory(token)
g = leaf(token, token, *[token], flag=token, **{'extra': token})
assert events == []
assert next(g) is token
assert events == ['entered']
assert next(g, token) is token
assert next(g, token) is token
class C:
    def leaf(self, value=token):
        events.append('method')
        yield value
c = C()
h = c.leaf()
assert events == ['entered']
assert next(h) is token
assert next(h, token) is token
g = h = c = leaf = factory = C = None
events = None
None
",
    );
    assert_eq!(output.counts["token"], 1);
}

/// Rejected requests release their input owners and leave dormant state retryable.
#[test]
fn rejected_first_send_and_reentry_release_inputs() {
    let output = checked_run(
        r#"
token = []
def leaf():
    try:
        g.send(token)
        assert False, 'expected reentry rejection'
    except ValueError as exc:
        assert str(exc) == 'generator already executing'
    yield token
    yield None
g = leaf()
try:
    g.send(token)
    assert False, 'expected first send rejection'
except TypeError as exc:
    assert str(exc) == "can't send non-None value to a just-started generator"
assert next(g) is token
assert g.send(token) is None
assert next(g, token) is token
assert next(g, token) is token
g = leaf = None
None
"#,
    );
    assert_eq!(output.counts["token"], 1);
}

/// Recursion rejection must preserve New/Suspended state and release input/default.
#[test]
fn recursion_admission_is_retryable() {
    for suspended in [false, true] {
        let source = format!(
            r"
token = []
def leaf():
    yield 1
    yield 2
g = leaf()
if {suspended}:
    assert next(g) == 1
def admit(n):
    if n:
        return admit(n - 1)
    try:
        if {suspended}:
            return g.send(token)
        return next(g, token)
    except RecursionError as exc:
        assert str(exc) == 'maximum recursion depth exceeded'
        return 'rejected'
assert admit(2) == 'rejected'
assert next(g) == {expected}
g = leaf = admit = None
None
",
            suspended = if suspended { "True" } else { "False" },
            expected = if suspended { 2 } else { 1 }
        );
        let mut run = MontyRun::new(source, "generator.py", vec![], CompileOptions::default()).unwrap();
        let output = run
            .run_ref_counts_with_tracker(
                vec![],
                ResourceTracker::new(ResourceLimits::default().max_recursion_depth(3)),
            )
            .unwrap();
        assert!(output.unreachable.is_empty(), "{:?}", output.unreachable);
        assert_eq!(output.counts["token"], 1);
    }
}

/// Yield delivery must root the result before releasing a temporary's last pin.
#[test]
fn temporary_last_pin_and_suspended_multiplicity() {
    let output = checked_run(
        r"
token = []
def leaf(a, b):
    x = a
    yield a
    assert x is b
assert next(leaf(token, token)) is token
g = leaf(token, token)
assert next(g) is token
None
",
    );
    // Global token plus the three dormant local slots a, b and x.
    assert_eq!(output.counts["token"], 4);
    assert_eq!(output.counts["g"], 1);
}

/// Trial deletion must reclaim New and Suspended cycles, including closure cells.
#[test]
fn dormant_cycles_are_collected_without_cleanup() {
    let output = checked_run(
        r"
events = []
def plain(a, b):
    try:
        yield a
    finally:
        events.append('cleanup')
def make(suspended):
    token = []
    g = plain(token, token)
    token.append(g)
    token.append(g)
    if suspended:
        next(g)
def cell_cycle():
    g = None
    def inner():
        yield g
    g = inner()
    next(g)
make(False)
make(True)
cell_cycle()
make = plain = cell_cycle = None
assert events == []
None
",
    );
    assert_eq!(output.heap_count, 1, "only the events list remains");
}

/// Exec-defined generators retain their namespace once, even in cyclic graphs.
#[test]
fn exec_namespace_cycles_are_collected() {
    let output = checked_run(
        r"
def make(suspended):
    ns = {}
    exec('def leaf(a, b):\n    yield a\n', ns)
    token = []
    g = ns['leaf'](token, token)
    ns['held'] = g
    token.append(ns)
    if suspended:
        next(g)
make(False)
make(True)
make = None
None
",
    );
    assert_eq!(output.heap_count, 0);
}

/// Caught exceptions and pending None returns survive an intervening yield.
#[test]
fn exception_stack_and_pending_return() {
    checked_run(
        r"
events = []
def leaf():
    try:
        raise ValueError('saved')
    except ValueError:
        yield 1
        raise
g = leaf()
assert next(g) == 1
try:
    next(g)
    assert False, 'expected bare raise'
except ValueError as exc:
    assert str(exc) == 'saved'
def pending():
    try:
        return None
    finally:
        yield 2
        events.append('finished')
h = pending()
assert next(h) == 2
assert next(h, 'done') == 'done'
assert events == ['finished']
def body_stop():
    yield 3
    raise StopIteration('body')
k = body_stop()
assert next(k) == 3
try:
    next(k)
    assert False, 'expected PEP 479 conversion'
except RuntimeError as exc:
    assert str(exc) == 'generator raised StopIteration'
g = h = k = leaf = pending = body_stop = None
None
",
    );
}

/// Original and restored active/dormant states abandon physically without finally.
#[test]
fn snapshots_resume_and_abandon_contextually() {
    for active in [true, false] {
        let mut repl = new_repl();
        repl.feed_run(
            r"
events = []
token = []
token.append(token)
def leaf(value):
    try:
        host()
        received = yield value
        assert received is value and value[0] is value
    finally:
        events.append('cleanup')
        yield value
g = leaf(token)
",
            vec![("host".to_owned(), MontyObject::function("host", None))],
            PrintWriter::Stdout,
        )
        .unwrap();
        let baseline = repl.heap_entry_count();
        let progress = repl.feed_start("next(g)", vec![], PrintWriter::Stdout).unwrap();
        let ReplProgress::FunctionCall(call) = progress else {
            panic!("active host cut")
        };
        let progress = if active {
            ReplProgress::FunctionCall(call)
        } else {
            let (repl, _) = call
                .resume(MontyObject::none(), PrintWriter::Stdout)
                .unwrap()
                .into_complete()
                .unwrap();
            repl.feed_start("host()", vec![], PrintWriter::Stdout).unwrap()
        };
        let bytes = dump("generator.py", None, &[], SessionRef::Suspended(&progress)).unwrap();
        let Session::Suspended(restored) = Dump::load(&bytes).unwrap().state else {
            panic!("restored cut")
        };
        for state in [progress, *restored] {
            let call = state.into_function_call().unwrap();
            let mut abandoned = call.into_repl();
            abandoned
                .feed_run(
                    "assert events == []\ng = None\ntoken = None",
                    vec![],
                    PrintWriter::Stdout,
                )
                .unwrap();
            assert_eq!(
                abandoned.heap_entry_count(),
                baseline - 2,
                "generator and cyclic token reclaimed before teardown"
            );
        }
        let Session::Suspended(restored) = Dump::load(&bytes).unwrap().state else {
            panic!("restored cut")
        };
        let (mut repl, _) = restored
            .into_function_call()
            .unwrap()
            .resume(MontyObject::none(), PrintWriter::Stdout)
            .unwrap()
            .into_complete()
            .unwrap();
        repl.feed_run("assert events == []\nassert g.send(token) is token\nassert events == ['cleanup']\nassert next(g, 'done') == 'done'", vec![], PrintWriter::Stdout).unwrap();
    }
}

/// The cancellation-type migration rejects the genuine v14 fixture.
#[test]
fn previous_snapshot_requires_old_runtime() {
    assert!(matches!(
        Dump::load(include_bytes!("fixtures/dump_v14.bin")),
        Err(monty::DumpError::VersionTooOld { .. })
    ));
}

/// Host errors unwind into yielding finally and remain pending across a dump.
#[test]
fn host_error_pending_across_cleanup_snapshot() {
    let repl = new_repl();
    let progress = repl
        .feed_start(
            r"
events = []
def leaf():
    try:
        host()
    finally:
        events.append('cleanup')
        yield None
        events.append('finished')
g = leaf()
next(g)
",
            vec![("host".to_owned(), MontyObject::function("host", None))],
            PrintWriter::Stdout,
        )
        .unwrap();
    let call = progress.into_function_call().unwrap();
    let (repl, _) = call
        .resume(
            MontyException::new(ExcType::ValueError, Some("host failure".to_owned())),
            PrintWriter::Stdout,
        )
        .unwrap()
        .into_complete()
        .unwrap();
    let progress = repl
        .feed_start("assert events == ['cleanup']\nhost()", vec![], PrintWriter::Stdout)
        .unwrap();
    let bytes = dump("generator.py", None, &[], SessionRef::Suspended(&progress)).unwrap();
    drop(progress.into_function_call().unwrap().into_repl());
    let Session::Suspended(progress) = Dump::load(&bytes).unwrap().state else {
        panic!("cleanup cut")
    };
    let (mut repl, _) = progress
        .into_function_call()
        .unwrap()
        .resume(MontyObject::none(), PrintWriter::Stdout)
        .unwrap()
        .into_complete()
        .unwrap();
    repl.feed_run(
        r"
try:
    next(g)
    assert False, 'expected pending host error'
except ValueError as exc:
    assert str(exc) == 'host failure'
assert events == ['cleanup', 'finished']
assert next(g, 'done') == 'done'
g = leaf = None
",
        vec![],
        PrintWriter::Stdout,
    )
    .unwrap();
}

/// Monty's existing asyncio.run route parks an active generator through a sync call.
#[test]
fn active_generator_in_parked_task() {
    let mut repl = new_repl();
    repl.feed_run(
        r"
import asyncio
events = []
token = []
async def waiting():
    return await host()
def ordinary():
    return asyncio.run(waiting())
def leaf():
    try:
        assert ordinary() == 7
        yield token
    finally:
        events.append('cleanup')
g = leaf()
async def task():
    assert next(g) is token
    return 11
async def sibling():
    return await host()
",
        vec![("host".to_owned(), MontyObject::function("host", None))],
        PrintWriter::Stdout,
    )
    .unwrap();
    let baseline = repl.heap_entry_count();
    let mut progress = repl
        .feed_start("await asyncio.gather(task(), sibling())", vec![], PrintWriter::Stdout)
        .unwrap();
    let mut ids = Vec::new();
    while let ReplProgress::FunctionCall(call) = progress {
        ids.push(call.call_id);
        progress = call.resume_pending(PrintWriter::Stdout).unwrap();
    }
    assert_eq!(ids.len(), 2);
    assert!(matches!(progress, ReplProgress::ResolveFutures(_)));
    let bytes = dump("generator.py", None, &[], SessionRef::Suspended(&progress)).unwrap();
    let Session::Suspended(restored) = Dump::load(&bytes).unwrap().state else {
        panic!("parked tasks")
    };
    for state in [progress, *restored] {
        let mut abandoned = state.into_resolve_futures().unwrap().into_repl();
        abandoned
            .feed_run("assert events == []\ng = token = None", vec![], PrintWriter::Stdout)
            .unwrap();
        assert_eq!(
            abandoned.heap_entry_count(),
            baseline - 2,
            "no parked owners survive abandonment"
        );
    }
    let Session::Suspended(restored) = Dump::load(&bytes).unwrap().state else {
        panic!("parked tasks")
    };
    let (mut repl, value) = restored
        .into_resolve_futures()
        .unwrap()
        .resume(
            ids.into_iter()
                .map(|id| (id, ExtFunctionResult::Return(MontyObject::int(7))))
                .collect(),
            PrintWriter::Stdout,
        )
        .unwrap()
        .into_complete()
        .unwrap();
    assert_eq!(value, MontyObject::list([MontyObject::int(11), MontyObject::int(7)]));
    repl.feed_run(
        "assert events == []\nassert next(g, 'done') == 'done'\nassert events == ['cleanup']",
        vec![],
        PrintWriter::Stdout,
    )
    .unwrap();
}

/// Forward send/throw through two delegates without a nested Rust run loop.
#[test]
fn nested_delegate_send_throw_and_direct_return() {
    let output = checked_run(
        r"
token = []
events = []
error = ValueError('injected')
def leaf():
    try:
        assert (yield token) is token
        yield token
    except ValueError as caught:
        assert caught is error
        yield token
        raise
    finally:
        events.append('cleanup')
        yield token
        events.append('finished')
def middle():
    return (yield from leaf())
def outer():
    return (yield from middle())
g = outer()
assert next(g) is token
assert g.send(token) is token
assert g.throw(error) is token
assert g.send(None) is token
try:
    next(g)
    assert False
except ValueError as caught:
    assert caught is error
assert events == ['cleanup', 'finished']
assert next(g, token) is token
def returning():
    yield None
    return token
def consume():
    result = yield from returning()
    assert result is token
    yield result
h = consume()
assert next(h) is None
assert next(h) is token
assert next(h, token) is token
g = h = error = outer = middle = leaf = consume = returning = events = None
None
",
    );
    assert_eq!(output.counts["token"], 1);
}

/// Invalid throw and recursive delegate admission preserve the dormant chain.
#[test]
fn throw_validation_and_delegate_admission() {
    let output = checked_run(
        r"
token = []
events = []
def leaf():
    try:
        yield token
    finally:
        events.append('cleanup')
        yield token
g = leaf()
try:
    g.throw(token)
    assert False
except TypeError:
    pass
assert events == []
assert next(g) is token
try:
    g.throw(token)
    assert False
except TypeError:
    pass
assert next(g, token) is token
assert events == ['cleanup']
assert next(g, token) is token
h = leaf()
try:
    h.throw(ValueError('new'))
    assert False
except ValueError as caught:
    assert str(caught) == 'new'
assert events == ['cleanup']
assert next(h, token) is token
try:
    h.throw(ValueError)
    assert False
except ValueError:
    pass
g = h = leaf = events = None
None
",
    );
    assert_eq!(output.counts["token"], 1);
}

/// The fixed completion program also checks aliases/rethrows and PEP 479.
#[test]
fn completion_identity_and_cycles() {
    let source = format!(
        "def exchange(value):\n    return None\n{}",
        include_str!("../../../tools/coroutines/cases/completion.py")
    );
    let output = checked_run(&source);
    assert_eq!(
        output.counts["token"], 2,
        "global and args tuple, never Closed generator"
    );
    let output = checked_run(
        r"
def make():
    token = []
    def leaf():
        yield None
        return token
    g = leaf()
    next(g)
    try:
        next(g)
    except StopIteration as done:
        assert done.args is done.args
        token.append(done)
    g = None
make()
make = None
None
",
    );
    assert_eq!(
        output.heap_count, 0,
        "completion/args/closure cycle reclaimed before teardown"
    );
}

/// All nested pins and pending thrown errors transfer through each cut; disposal
/// of either copy releases them without authored cleanup.
#[test]
fn nested_snapshots_throw_and_abandonment() {
    for stage in 0..3 {
        let mut repl = new_repl();
        repl.feed_run(
            r"
events = []
token = []
token.append(token)
error = ValueError('injected')
def leaf(value):
    try:
        host()
        yield value
    finally:
        events.append('cleanup')
        yield value
        events.append('finished')
def middle(value):
    return (yield from leaf(value))
def outer(value):
    return (yield from middle(value))
g = outer(token)
",
            vec![("host".to_owned(), MontyObject::function("host", None))],
            PrintWriter::Stdout,
        )
        .unwrap();
        let baseline = repl.heap_entry_count();
        let mut progress = repl.feed_start("next(g)", vec![], PrintWriter::Stdout).unwrap();
        if stage > 0 {
            let (mut repl, _) = progress
                .into_function_call()
                .unwrap()
                .resume(MontyObject::none(), PrintWriter::Stdout)
                .unwrap()
                .into_complete()
                .unwrap();
            if stage == 2 {
                repl.feed_run("assert g.throw(error) is token", vec![], PrintWriter::Stdout)
                    .unwrap();
            }
            progress = repl.feed_start("host()", vec![], PrintWriter::Stdout).unwrap();
        }
        let bytes = dump("generator.py", None, &[], SessionRef::Suspended(&progress)).unwrap();
        let Session::Suspended(restored) = Dump::load(&bytes).unwrap().state else {
            panic!("cut")
        };
        for state in [progress, *restored] {
            let mut abandoned = state.into_function_call().unwrap().into_repl();
            abandoned
                .feed_run(
                    if stage == 2 {
                        "assert events == ['cleanup']\ng = token = error = None"
                    } else {
                        "assert events == []\ng = token = error = None"
                    },
                    vec![],
                    PrintWriter::Stdout,
                )
                .unwrap();
            assert_eq!(
                abandoned.heap_entry_count(),
                baseline - 3,
                "stage {stage}: all delegate/error owners released"
            );
        }
        let Session::Suspended(restored) = Dump::load(&bytes).unwrap().state else {
            panic!("cut")
        };
        let (mut repl, _) = restored
            .into_function_call()
            .unwrap()
            .resume(MontyObject::none(), PrintWriter::Stdout)
            .unwrap()
            .into_complete()
            .unwrap();
        if stage < 2 {
            repl.feed_run("assert g.throw(error) is token", vec![], PrintWriter::Stdout)
                .unwrap();
        }
        repl.feed_run(
            "try:\n    next(g)\n    assert False\nexcept ValueError as caught:\n    assert caught is error\nassert events == ['cleanup', 'finished']\ng = token = error = None",
            vec![], PrintWriter::Stdout,
        ).unwrap();
        assert_eq!(repl.heap_entry_count(), baseline - 3);
    }
}

/// Fallible export releases pinned children and every remaining argument before
/// returning a public diagnostic. No partial host event is published.
#[test]
fn boundary_rejection_releases_graphs_and_arguments() {
    for expression in [
        "[clean, {'nested': (clean, saved)}, token]",
        "host(clean, saved, token, left=clean, right=token)",
        "host(clean, left=clean, reject=saved, right=token)",
        "StopIteration()",
        "StopIteration(42)",
        "receiver.method(clean, saved, token, left=clean, right=token)",
    ] {
        let mut repl = new_repl();
        repl.feed_run(
            "token = []\nsaved = StopIteration(token)\ntoken.append(saved)\nclean = []",
            vec![
                ("host".to_owned(), MontyObject::function("host", None)),
                (
                    "receiver".to_owned(),
                    MontyObject::class_instance(
                        MontyObject::class_type("Receiver", MontyUuid::from_u128(1), true, false, []),
                        MontyUuid::from_u128(2),
                        [],
                    ),
                ),
            ],
            PrintWriter::Stdout,
        )
        .unwrap();
        let baseline = repl.heap_entry_count();
        let failure = repl.feed_start(expression, vec![], PrintWriter::Stdout).unwrap_err();
        assert_eq!(failure.error.exc_type(), ExcType::TypeError);
        assert_eq!(
            failure.error.message(),
            Some("cannot export exception with owned args to the host")
        );
        let mut repl = failure.repl;
        assert_eq!(repl.heap_entry_count(), baseline, "{expression}");
        repl.feed_run("token = saved = clean = None\n[]", vec![], PrintWriter::Stdout)
            .unwrap();
        assert_eq!(
            repl.heap_entry_count(),
            baseline - 4,
            "cycle, earlier pinned alias and every remaining argument released"
        );
    }
    // Escaped runtime diagnostics preserve metadata and do not export payloads.
    let mut repl = new_repl();
    repl.feed_run(
        "token = []\nsaved = ValueError(token)\ntoken.append(saved)",
        vec![],
        PrintWriter::Stdout,
    )
    .unwrap();
    let baseline = repl.heap_entry_count();
    repl.feed_run("ValueError('legacy')", vec![], PrintWriter::Stdout)
        .unwrap();
    let error = repl.feed_run("raise saved", vec![], PrintWriter::Stdout).unwrap_err();
    assert_eq!(error.exc_type(), ExcType::ValueError);
    assert_eq!(error.message(), None);
    repl.feed_run("token = saved = None\n[]", vec![], PrintWriter::Stdout)
        .unwrap();
    assert_eq!(repl.heap_entry_count(), baseline - 3);
}

/// Cached gather failures and swallowed/replaced errors keep exactly their
/// intended roots, including an args cycle and repeated failure delivery.
#[test]
fn cached_failures_and_discarded_errors_release_owners() {
    let output = checked_run(
        r"
import asyncio
token = []
saved = ValueError(token)
token.append(saved)
async def child():
    raise saved
async def drive():
    gathered = asyncio.gather(child())
    for attempt in [0, 1, 2]:
        try:
            await gathered
            assert False
        except ValueError as caught:
            assert caught is saved and caught.args[0] is token
asyncio.run(drive())
class Iterable:
    def __iter__(self):
        return self
    def __next__(self):
        raise StopIteration(token)
assert list(Iterable()) == []
assert next(iter(lambda: next(Iterable()), None), 'done') == 'done'
class Bad:
    def __repr__(self):
        raise saved
try:
    assert Bad() == 2
except AssertionError:
    pass
saved = token = child = drive = Iterable = Bad = None
None
",
    );
    assert!(output.unreachable.is_empty());
}

/// A reservation failure partway through a saved delegate chain rolls back all
/// charges and leaves every dormant state intact, including rejected throw args.
#[test]
fn nested_admission_rolls_back_before_state_moves() {
    let mut run = MontyRun::new(
        r"
token = []
def leaf():
    yield token
    yield token
def middle():
    yield from leaf()
def outer():
    yield from middle()
g = outer()
assert next(g) is token
def admit(n):
    if n:
        return admit(n - 1)
    try:
        g.throw(ValueError(token))
    except RecursionError:
        return 'rejected'
assert admit(2) == 'rejected'
assert g.send(None) is token
assert next(g, 'done') == 'done'
g = leaf = middle = outer = admit = None
None
"
        .to_owned(),
        "generator.py",
        vec![],
        CompileOptions::default(),
    )
    .unwrap();
    let output = run
        .run_ref_counts_with_tracker(
            vec![],
            ResourceTracker::new(ResourceLimits::default().max_recursion_depth(5)),
        )
        .unwrap();
    assert!(output.unreachable.is_empty());
    assert_eq!(output.counts["token"], 1);
}

/// A throw replaces a pending owning return, and a finally return suppresses
/// a pending owning error without retaining either abandoned payload.
#[test]
fn pending_completion_and_error_replacement() {
    let output = checked_run(
        r"
token = []
error = ValueError(token)
def returning():
    try:
        return token
    finally:
        yield token
g = returning()
assert next(g) is token
try:
    g.throw(error)
    assert False
except ValueError as caught:
    assert caught is error
assert next(g, 'done') == 'done'
def suppressing():
    try:
        raise error
    finally:
        yield token
        return token
h = suppressing()
assert next(h) is token
try:
    next(h)
    assert False
except StopIteration as done:
    assert done.value is token
    assert done.args[0] is token
g = h = error = returning = suppressing = None
None
",
    );
    assert_eq!(output.counts["token"], 1);
}

/// Host diagnostics materialize one cached identity, shared with a caught alias
/// through an active snapshot and repeated await, then release before teardown.
#[test]
fn host_future_error_identity_survives_snapshot_and_reawait() {
    let mut repl = new_repl();
    repl.feed_run(
        r"
import asyncio
async def drive():
    try:
        await future
        assert False
    except ValueError as caught:
        saved = caught
    host()
    for attempt in [0, 1, 2]:
        try:
            await future
            assert False
        except ValueError as caught:
            assert caught is saved, 'cached external error identity'
            assert str(caught) == 'host failure'
",
        vec![("host".to_owned(), MontyObject::function("host", None))],
        PrintWriter::Stdout,
    )
    .unwrap();
    let baseline = repl.heap_entry_count();
    let call = repl
        .feed_start("future = host()\nawait drive()", vec![], PrintWriter::Stdout)
        .unwrap()
        .into_function_call()
        .unwrap();
    let id = call.call_id;
    let progress = call
        .resume_pending(PrintWriter::Stdout)
        .unwrap()
        .into_resolve_futures()
        .unwrap()
        .resume(
            vec![(
                id,
                ExtFunctionResult::Error(MontyException::new(
                    ExcType::ValueError,
                    Some("host failure".to_owned()),
                )),
            )],
            PrintWriter::Stdout,
        )
        .unwrap();
    let bytes = dump("generator.py", None, &[], SessionRef::Suspended(&progress)).unwrap();
    let mut original = Some(progress);
    for restore in [false, true] {
        let state = if restore {
            let Session::Suspended(restored) = Dump::load(&bytes).unwrap().state else {
                panic!("cached cut")
            };
            *restored
        } else {
            original.take().unwrap()
        };
        let (mut repl, _) = state
            .into_function_call()
            .unwrap()
            .resume(MontyObject::none(), PrintWriter::Stdout)
            .unwrap()
            .into_complete()
            .unwrap();
        repl.feed_run("future = None\n[]", vec![], PrintWriter::Stdout).unwrap();
        assert_eq!(
            repl.heap_entry_count(),
            baseline,
            "cached error and all reawait aliases released before teardown"
        );
    }
}

/// Owned argument formatting must charge the VM recursion budget and release
/// every temporary argument clone when either str or repr reaches the limit.
#[test]
fn exception_argument_formatting_respects_recursion_budget() {
    let mut run = MontyRun::new(
        r"
e = StopIteration('root')
for attempt in range(40):
    e = StopIteration(e)
try:
    str(e)
    assert False, 'expected bounded exception str'
except RecursionError:
    pass
try:
    repr(e)
    assert False, 'expected bounded exception repr'
except RecursionError:
    pass
e = None
None
"
        .to_owned(),
        "generator.py",
        vec![],
        CompileOptions::default(),
    )
    .unwrap();
    let output = run
        .run_ref_counts_with_tracker(
            vec![],
            ResourceTracker::new(ResourceLimits::default().max_recursion_depth(8)),
        )
        .unwrap();
    assert!(output.unreachable.is_empty());
    assert_eq!(
        output.heap_count, 0,
        "argument chains and formatting clones released before teardown"
    );
}

/// A delegate exhausted through another alias reports StopIteration from throw
/// as delegation completion, not as a StopIteration raised by a running body.
#[test]
fn closed_delegate_throw_stop_preserves_completion_identity() {
    let output = checked_run(
        r"
token = []
def leaf():
    yield token
    return token
def outer(child):
    return (yield from child)
child = leaf()
g = outer(child)
assert next(g) is token
assert next(child, 'done') == 'done'
injected = StopIteration(token)
try:
    g.throw(injected)
    assert False, 'expected delegated completion'
except StopIteration as done:
    assert done is not injected
    assert done.value is token and done.args[0] is token
assert next(g, 'done') == 'done'
g = child = injected = leaf = outer = None
None
",
    );
    assert_eq!(output.counts["token"], 1);
}

/// A Running alias fails at the saved yield-from site, with all dormant
/// ancestors installed. Catches, rethrows and cleanup replacement keep identity.
#[test]
fn running_descendant_unwind_releases_owners() {
    for mode in ["caught", "unhandled", "replaced", "cleanup"] {
        let source = running_descendant_source(mode, "pass");
        let output = checked_run(&source);
        assert_eq!(output.counts["token"], 1, "{mode}");
        assert_eq!(output.counts["seen"], 1, "{mode}");
    }
}

/// Active host snapshots retain alias pins; both original and restored copies
/// run ancestor unwind and release every generator/error owner before teardown.
#[test]
fn running_descendant_unwind_after_active_restore() {
    for mode in ["caught", "unhandled", "replaced", "cleanup"] {
        let mut repl = new_repl();
        repl.feed_run(
            "token = []\nseen = []",
            vec![("host".to_owned(), MontyObject::function("host", None))],
            PrintWriter::Stdout,
        )
        .unwrap();
        let baseline = repl.heap_entry_count();
        let source = running_descendant_source(mode, "host()");
        let progress = repl
            .feed_start(
                &source,
                vec![("host".to_owned(), MontyObject::function("host", None))],
                PrintWriter::Stdout,
            )
            .unwrap();
        let bytes = dump("generator.py", None, &[], SessionRef::Suspended(&progress)).unwrap();
        let (mut original, _) = progress
            .into_function_call()
            .unwrap()
            .resume(MontyObject::none(), PrintWriter::Stdout)
            .unwrap()
            .into_complete()
            .unwrap();
        original.feed_run("[]", vec![], PrintWriter::Stdout).unwrap();
        assert_eq!(
            original.heap_entry_count(),
            baseline + 1,
            "original {mode}: only events remains"
        );
        let Session::Suspended(restored) = Dump::load(&bytes).unwrap().state else {
            panic!("active cut")
        };
        let (mut restored, _) = restored
            .into_function_call()
            .unwrap()
            .resume(MontyObject::none(), PrintWriter::Stdout)
            .unwrap()
            .into_complete()
            .unwrap();
        restored.feed_run("[]", vec![], PrintWriter::Stdout).unwrap();
        assert_eq!(
            restored.heap_entry_count(),
            baseline + 1,
            "restored {mode}: only events remains"
        );
    }
}

/// Terminal errors from guest formatting cannot publish a result/event or enter
/// guest handlers. Transaction pins, temporary partials and remaining args drain.
#[test]
fn terminal_formatting_fallbacks_release_owners() {
    for expression in [
        "{}[Bad()]",
        "repr(Bad())",
        "functools.partial(f, Bad())",
        "[clean, functools.partial(f, Bad()), token]",
        "host(clean, functools.partial(f, Bad()), token, left=clean, right=token)",
        "host(clean, left=clean, reject=functools.partial(f, Bad()), right=token)",
        "assert Bad() == 1",
    ] {
        let mut repl = new_repl();
        repl.feed_run(
            r"
import functools
import os
token = []
saved = StopIteration(token)
token.append(saved)
clean = []
events = []
class Bad:
    def __repr__(self):
        os.getenv('x', saved)
        events.append('repr continued')
        return 'unreachable'
def f(x):
    return x
",
            vec![("host".to_owned(), MontyObject::function("host", None))],
            PrintWriter::Stdout,
        )
        .unwrap();
        let baseline = repl.heap_entry_count();
        let source = if expression.starts_with("functools.partial") || expression.starts_with("[clean") {
            expression.to_owned()
        } else {
            format!(
                "try:\n    {expression}\nexcept (TypeError, KeyError, AssertionError):\n    events.append('caught')\n"
            )
        };
        let failure = repl.feed_start(&source, vec![], PrintWriter::Stdout).unwrap_err();
        assert_eq!(failure.error.exc_type(), ExcType::TypeError, "{expression}");
        assert_eq!(
            failure.error.message(),
            Some("cannot export exception with owned args to the host")
        );
        let mut repl = failure.repl;
        assert_eq!(repl.heap_entry_count(), baseline, "{expression}");
        repl.feed_run(
            "assert events == []\ntoken = saved = clean = None\n[]",
            vec![],
            PrintWriter::Stdout,
        )
        .unwrap();
        assert_eq!(repl.heap_entry_count(), baseline - 4, "{expression}");
    }
    // Ordinary owning repr failures still use the established catchable fallback.
    let output = checked_run(
        r#"
token = []
error = ValueError(token)
class Bad:
    def __repr__(self):
        raise error
try:
    {}[Bad()]
    assert False, 'expected KeyError fallback'
except KeyError as caught:
    assert str(caught) == "'<Bad>'"
try:
    assert list(range(500)) == []
    assert False, 'expected truncated assertion detail'
except AssertionError:
    pass
error = Bad = None
None
"#,
    );
    assert_eq!(output.counts["token"], 1);
    let mut repl = new_repl();
    let result = repl
        .feed_run(
            r"
import functools
token = []
error = ValueError(token)
class Bad:
    def __repr__(self):
        raise error
def f(x):
    return x
functools.partial(f, Bad())
",
            vec![],
            PrintWriter::Stdout,
        )
        .unwrap();
    assert_eq!(
        result,
        MontyObject::repr("<functools.partial object, error on repr(): ValueError()>")
    );
    let baseline = repl.heap_entry_count();
    repl.feed_run("token = error = None\n[]", vec![], PrintWriter::Stdout)
        .unwrap();
    assert_eq!(repl.heap_entry_count(), baseline - 3);
}

/// Cyclic containers, rather than the exception wrapper, terminate exception
/// repr. Repeated aliases keep their type and all cycles remain collectable.
#[test]
fn cyclic_exception_repr_preserves_wrappers() {
    let output = checked_run(
        r"
x = []
e = StopIteration(x)
x.append(e)
assert repr(e) == 'StopIteration([StopIteration([...])])'
assert repr([e, e]) == '[StopIteration([StopIteration([...])]), StopIteration([StopIteration([...])])]'
y = []
f = ValueError(y)
y.append(f)
assert repr(f) == 'ValueError([ValueError([...])])'
assert e.args[0] is x and x[0] is e
assert f.args[0] is y and y[0] is f
x = e = y = f = None
None
",
    );
    assert_eq!(output.heap_count, 0);
}

/// Shared alias/unwind program with an optional active host cut.
fn running_descendant_source(mode: &str, cut: &str) -> String {
    format!(
        r"
mode = '{mode}'
token = []
seen = []
events = []
injected = ValueError(token)
replacement = TypeError(token)
def child():
    yield token
    {cut}
    try:
        assert g.throw(injected) is token
        assert mode == 'caught' or mode == 'cleanup'
    except ValueError as caught:
        assert mode == 'unhandled' and caught is seen[0]
    except TypeError as caught:
        assert mode == 'replaced' and caught is replacement
    yield token
def parent():
    try:
        yield from c
    except ValueError as caught:
        assert caught is not injected
        assert str(caught) == 'generator already executing'
        seen.append(caught)
        if mode == 'caught':
            yield token
        elif mode == 'replaced':
            raise replacement
        else:
            raise
    finally:
        events.append('parent')
        if mode == 'cleanup':
            yield token
def outer():
    try:
        yield from parent()
    finally:
        events.append('outer')
c = child()
g = outer()
assert next(g) is token
assert next(c) is token
if mode == 'cleanup':
    try:
        next(g)
        assert False, 'expected saved reentry error'
    except ValueError as caught:
        assert caught is seen[0]
else:
    assert next(g, 'done') == 'done'
assert events == ['parent', 'outer']
assert next(c, 'done') == 'done'
g = c = child = parent = outer = injected = replacement = None
seen.clear()
None
"
    )
}

/// Inspect all live owners after GC and before globals or the heap are destroyed.
fn checked_run(source: &str) -> monty::RefCountOutput {
    let mut run = MontyRun::new(source.to_owned(), "generator.py", vec![], CompileOptions::default()).unwrap();
    let output = run.run_ref_counts(vec![]).unwrap();
    assert!(
        output.unreachable.is_empty(),
        "unreachable owners: {:?}",
        output.unreachable
    );
    output
}

/// A low GC interval makes abandonment collection observable between feeds.
fn new_repl() -> MontyRepl {
    MontyRepl::new(
        "generator.py",
        ResourceTracker::new(ResourceLimits::default().gc_interval(1)),
        CompileOptions::default(),
    )
}

/// Count the actual scope-library task/frame graph before teardown, including a
/// host cut inside an executing child (activation pin) and dormant phase cuts.
#[test]
fn scope9_aggregate_original_and_restored_release_before_teardown() {
    let library = include_str!("../../../tools/coroutines/scope7_guest.py");
    let helpers = include_str!("../../../tools/coroutines/scope9_ownership.py");
    let stages = [
        ("body-wait", "start9()\nhost()"),
        (
            "queued-acquire",
            "step9()\nstep9()\nstep9()\nstep9('T1')\nsettle9('T1', token)\nhost()",
        ),
        ("queued-value", "start9()\nsettle9('T1', token)\nhost()"),
        ("queued-error", "start9()\nsettle9('T1', token, 'error')\nhost()"),
        (
            "queued-join",
            "start9()\nsettle9('T1', 'failed', 'error')\nfinish9('T1')\nassert state['tasks']['T0']['resume'][1] is state['tasks']['T1']['result']\nhost()",
        ),
        ("drain", "start9()\ncancel9()\nstep9()\nhost()"),
        (
            "child-cleanup",
            "start9()\ncancel9()\nstep9()\nstep9('T1')\nstep9('T1')\nassert state['tasks']['T1']['deferred'][0][1] is token\nhost()",
        ),
        (
            "parent-cleanup",
            "start9()\ncancel9()\nstep9()\nfinish9('T1')\nfinish9('T2')\nstep9()\nhost()",
        ),
        ("active-child", "step9()\nstep9()\nstep9()\nstep9('T1')"),
    ];
    for (stage, cut) in stages {
        let mut repl = new_repl();
        let library = if stage == "active-child" {
            library.replace(
                "    hold = yield",
                "    global checkpoint9\n    checkpoint9 = state['events'][:]\n    scope_probe()\n    hold = yield",
            )
        } else {
            library.to_owned()
        };
        repl.feed_run(
            &format!("{library}\n{helpers}\ntoken = []\ncheckpoint9 = None\nstate = None"),
            vec![
                ("host".to_owned(), MontyObject::function("host", None)),
                ("scope_probe".to_owned(), MontyObject::function("scope_probe", None)),
            ],
            PrintWriter::Stdout,
        )
        .unwrap();
        let baseline = repl.heap_entry_count();
        let progress = repl
            .feed_start(
                &format!(
                    "state = new_driver('normal')\n{}",
                    cut.replace("host()", "checkpoint9 = state['events'][:]\nhost()")
                ),
                vec![],
                PrintWriter::Stdout,
            )
            .unwrap();
        assert!(matches!(progress, ReplProgress::FunctionCall(_)), "{stage}");
        let bytes = dump("generator.py", None, &[], SessionRef::Suspended(&progress)).unwrap();
        let Session::Suspended(restored) = Dump::load(&bytes).unwrap().state else {
            panic!("{stage}: suspended aggregate required")
        };
        let mut measurements = Vec::new();
        for (kind, progress) in [("original", progress), ("restored", *restored)] {
            // into_repl abandons suspended VM owners without executing guest code;
            // retain the heap to measure collection rather than just process exit.
            let mut abandoned = progress.into_function_call().unwrap().into_repl();
            abandoned
                .feed_run(
                    "assert state['events'] == checkpoint9\ncheckpoint9 = None\nstate = None\nassert checkpoint9 is None and state is None",
                    vec![],
                    PrintWriter::Stdout,
                )
                .unwrap();
            let first = abandoned.heap_entry_count();
            // Collection is allocation-triggered, not forced by an allocation-
            // free feed. Tick the existing collector after releasing all roots.
            abandoned
                .feed_run("pulse9 = []\npulse9 = None", vec![], PrintWriter::Stdout)
                .unwrap();
            measurements.push((kind, first, abandoned.heap_entry_count()));
        }
        for (kind, first, final_count) in measurements {
            assert_eq!(final_count, baseline, "{stage}/{kind}: first={first}, before teardown");
        }
    }
}

/// Inspect live handle/result owners as well as final reclamation. Queued owned
/// values/errors must retire on cancellation; rejected duplicate replies cannot
/// replace them. Event history is deliberately cleared for lifetime measurement.
#[test]
fn scope9_queued_owners_deferred_handles_and_retained_results() {
    let library = include_str!("../../../tools/coroutines/scope7_guest.py");
    let helpers = include_str!("../../../tools/coroutines/scope9_ownership.py");
    let output = checked_run(&format!(
        "{library}\n{helpers}\n{}",
        r"
token = []
state = new_driver('normal')
start9()
assert state['tasks']['T1']['deferred'][0][1] is token
settle9('T1', token)
command(state, {'run':'R7', 'kind':'settle', 'task':'T1', 'boundary':'wrong', 'outcome':'error', 'value':token})
assert state['tasks']['T1']['resume'][1] is token
cancel9()
step9()
step9('T1')
step9('T1')
assert state['tasks']['T1']['deferred'][0][1] is token
wait = state['tasks']['T1']['wait']
cancel9()
assert state['tasks']['T1']['wait'] is wait
settle9('T1')
step9('T1')
finish9('T2')
step9()
settle9('T0')
step9()
assert state['tasks']['T0']['result']['kind'] == 'cancelled'
state = wait = None
None
"
    ));
    assert_eq!(output.counts["token"], 1, "only the global handle survives");
    // The same aggregate with successful results must retain an intentional alias
    // until the task table and event history are both released.
    let result_program = format!(
        "{library}\n{helpers}\n{}",
        r"
token = []
state = new_driver('normal')
start9()
settle9('T1', token)
finish9('T1')
assert state['tasks']['T0']['resume'][1] is state['tasks']['T1']['result']
step9()
settle9('T2', token)
finish9('T2')
step9()
step9()
settle9('T0')
step9()
retained = state['tasks']['T0']['result']['value'][0]
assert retained['value'] is token
state = None
None
"
    );
    let output = checked_run(&result_program);
    assert_eq!(output.counts["token"], 2, "global plus intentionally retained result");
    let output = checked_run(&format!("{result_program}\nretained = None\nNone"));
    assert_eq!(
        output.counts["token"], 1,
        "last retained result released before teardown"
    );
    let output = checked_run(&format!(
        "{library}\n{helpers}\n{}",
        r"
token = []
state = new_driver('normal')
start9()
settle9('T1', token, 'error')
cancel9()
state = None
None
"
    ));
    assert_eq!(output.counts["token"], 1, "queued error and cyclic frames released");
}

/// Host syntax uses the same owner graph through pending returns and exceptions.
#[test]
fn host_async_context_normal_error_return_and_suppression() {
    let library = include_str!("../../../tools/coroutines/async10_guest.py");
    let source = format!(
        "{library}\n{}",
        r"
token = []
token.append(token)
for mode in ['normal', 'return', 'error', 'suppress', 'catch']:
    events = []
    failures = []
    g = cart(events, failures, mode)
    assert events == []
    assert next(g) == ['scope-enter', 50]
    assert g.send(None) == ['acquire']
    request = g.send(token)
    if mode == 'normal' or mode == 'catch':
        assert request == ['use', token]
        request = g.throw(ValueError('cancel')) if mode == 'catch' else g.send(token)
    assert request == ['audit-release', token]
    assert g.throw(ValueError('cleanup-failed')) == ['release', token]
    try:
        g.send(None)
        assert False
    except StopIteration as done:
        if mode == 'suppress':
            assert done.value == 'suppressed'
        elif mode == 'catch':
            assert done.value == 'caught'
        else:
            assert done.value is token
    except ValueError as error:
        assert mode == 'error' and str(error) == 'body-failed'
    assert failures == ['cleanup-failed']
    assert len(events) == 2
    g = events = failures = request = None
cart = Timebox = effect = None
None
"
    );
    let output = checked_run(&source);
    assert_eq!(output.counts["token"], 2, "one global and one self edge");
}

/// Original/restored context and continuation cycles are released before teardown.
#[test]
fn host_async_context_original_restored_disposal() {
    let setup = r"
from __monty_host__ import host_coroutines
events = []
token = None
g = None
checkpoint = None
cut_at = None

def pause(where):
    if cut_at == where:
        global checkpoint
        checkpoint = events[:]
        host()
    return (yield token)

class C:
    def __init__(self, value):
        self.value = value
    async def __aenter__(self):
        await pause('entry')
        return self
    async def __aexit__(self, typ, value, tb):
        assert tb is None
        if value is not None:
            assert value.args[0] is token
        events.append('exit')
        await pause('exit')
        events.append('finished')
        return False

async def work(mode):
    async with C(token) as scope:
        assert scope.value is token
        if mode == 'return':
            return token
        await pause('body')
";
    for (stage, cut) in [
        ("new", "g = work('return')"),
        ("entry", "g = work('return')\nassert next(g) is token"),
        ("body", "g = work('body')\nnext(g)\nassert g.send(None) is token"),
        ("return", "g = work('return')\nnext(g)\nassert g.send(None) is token"),
        (
            "error",
            "g = work('body')\nnext(g)\ng.send(None)\nassert g.throw(ValueError(token)) is token",
        ),
        ("active-entry", "cut_at = 'entry'\ng = work('body')\nnext(g)"),
        (
            "active-exit",
            "cut_at = 'exit'\ng = work('body')\nnext(g)\ng.send(None)\ng.throw(ValueError(token))",
        ),
    ] {
        let mut repl = new_repl();
        repl.feed_run(
            setup,
            vec![("host".to_owned(), MontyObject::function("host", None))],
            PrintWriter::Stdout,
        )
        .unwrap();
        let baseline = repl.heap_entry_count();
        let end = if stage.starts_with("active-") {
            ""
        } else {
            "\ncheckpoint = events[:]\nhost()"
        };
        let source = format!("token = []\ntoken.append(token)\n{cut}{end}");
        let progress = repl.feed_start(&source, vec![], PrintWriter::Stdout).unwrap();
        assert!(matches!(progress, ReplProgress::FunctionCall(_)), "{stage}");
        let bytes = dump("host.py", None, &[], SessionRef::Suspended(&progress)).unwrap();
        let Session::Suspended(restored) = Dump::load(&bytes).unwrap().state else {
            panic!("suspended")
        };
        for (kind, progress) in [("original", progress), ("restored", *restored)] {
            let mut repl = progress.into_function_call().unwrap().into_repl();
            repl.feed_run("assert events == checkpoint\ng = token = checkpoint = cut_at = None\nevents = []\npulse = []\npulse = None", vec![], PrintWriter::Stdout).unwrap();
            assert_eq!(repl.heap_entry_count(), baseline, "{stage}/{kind}: before teardown");
        }
    }
}

/// A host-dialect compile must not replace default Coroutine/asyncio semantics.
#[test]
fn native_async_scheduler_still_works() {
    checked_run(
        r"
import asyncio
async def leaf(value):
    return value
async def main():
    return await asyncio.gather(leaf(1), leaf(2))
assert asyncio.run(main()) == [1, 2]
None
",
    );
}

/// Normal fallthrough, loop exits, nested managers and protected target binding.
#[test]
fn host_async_context_nonlocal_and_binding_paths() {
    checked_run(
        r"
from __monty_host__ import host_coroutines
events = []
def effect(value):
    yield value
class C:
    def __init__(self, label):
        self.label = label
    async def __aenter__(self):
        return [1, 2]
    async def __aexit__(self, typ, value, tb):
        events.append(self.label)
        await effect(self.label)
        return False
async def work(mode):
    for i in [1, 2]:
        async with C('a') as (x, y), C('b'):
            assert x == 1 and y == 2
            if mode == 'continue':
                continue
            if mode == 'break':
                break
    return 'done'
for mode in ['normal', 'continue', 'break']:
    events = []
    g = work(mode)
    assert next(g) == 'b'
    assert g.send(None) == 'a'
    if mode != 'break':
        assert g.send(None) == 'b'
        assert g.send(None) == 'a'
    try:
        g.send(None)
        assert False
    except StopIteration as done:
        assert done.value == 'done'
    assert events == (['b', 'a'] if mode == 'break' else ['b', 'a', 'b', 'a'])
async def bad_target():
    async with C('unpack') as (a, b, c):
        assert False
g = bad_target()
assert next(g) == 'unpack'
try:
    g.send(None)
    assert False
except ValueError:
    pass
assert events[-1] == 'unpack'
None
",
    );
}

/// Combined scopes own pending completions, deferred handles and cyclic task frames.
#[test]
fn combined11_original_restored_abandon_before_teardown() {
    let library = include_str!("../../../tools/coroutines/combined11_guest.py");
    let driver = include_str!("../../../tools/coroutines/combined11_driver.py");
    let helpers = include_str!("../../../tools/coroutines/combined11_ownership.py");
    for (stage, cut) in [
        ("new", "host()"),
        ("body", "start11()\nhost()"),
        (
            "queued-acquire",
            "step11()\nstep11()\nstep11()\nstep11('T1')\nsettle11('T1', token)\nhost()",
        ),
        ("queued-value", "start11()\nsettle11('T1', token)\nhost()"),
        ("queued-error", "start11()\nsettle11('T1', token, 'error')\nhost()"),
        (
            "cancel-overwrite",
            "start11()\nsettle11('T1', token)\ncancel11()\nhost()",
        ),
        ("child-drain", "start11()\ncancel11()\nstep11()\nstep11('T1')\nhost()"),
        (
            "child-cleanup",
            "start11()\ncancel11()\nstep11()\nstep11('T1')\nstep11('T1')\nhost()",
        ),
        (
            "queued-cleanup",
            "start11()\ncancel11()\nstep11()\nstep11('T1')\nstep11('T1')\nsettle11('T1', token)\ncancel11('repeat')\nhost()",
        ),
        (
            "parent-cleanup",
            "start11()\ncancel11()\nstep11()\nretire11('T1')\nretire11('T2')\nstep11()\nhost()",
        ),
        (
            "pending-return",
            "state = new_driver('early')\nstep11()\nstep11()\nstep11()\nhost()",
        ),
        (
            "pending-error",
            "state = new_driver('error')\nstart11()\nsettle11(value=token, outcome='error')\nstep11()\nhost()",
        ),
        (
            "queued-join",
            "start11()\nsettle11('T1', token)\nretire11('T1')\nhost()",
        ),
        (
            "pending-result-alias",
            "start11()\nsettle11('T1', token)\nretire11('T1')\nsettle11('T2', token)\nretire11('T2')\nstep11()\nstep11()\nhost()",
        ),
        (
            "nested-exit",
            "state = new_driver('nested')\nstep11()\nsettle11(value=token)\nstep11()\nstep11()\nhost()",
        ),
        ("active-child", "step11()\nstep11()\nstep11()\nstep11('T1')"),
    ] {
        let mut repl = new_repl();
        let library = if stage == "active-child" {
            library.replace(
                "        hold = await tg.operation",
                "        global checkpoint11\n        checkpoint11 = state['events'][:]\n        host()\n        hold = await tg.operation",
            )
        } else {
            library.to_owned()
        };
        let setup = format!(
            "{library}\n{driver}\n{helpers}\ndef adapt(body):\n    return body\nstate = token = checkpoint11 = None"
        );
        repl.feed_run(
            &setup,
            vec![("host".to_owned(), MontyObject::function("host", None))],
            PrintWriter::Stdout,
        )
        .unwrap();
        let baseline = repl.heap_entry_count();
        let checkpoint = if stage == "active-child" {
            ""
        } else {
            "\ncheckpoint11 = state['events'][:]"
        };
        let source = format!(
            "token = []\ntoken.append(token)\nstate = new_driver('normal')\n{}",
            cut.replace("host()", &format!("{checkpoint}\nhost()"))
        );
        let progress = repl.feed_start(&source, vec![], PrintWriter::Stdout).unwrap();
        assert!(matches!(progress, ReplProgress::FunctionCall(_)), "{stage}");
        let bytes = dump("host.py", None, &[], SessionRef::Suspended(&progress)).unwrap();
        let Session::Suspended(restored) = Dump::load(&bytes).unwrap().state else {
            panic!("suspended")
        };
        for (kind, progress) in [("original", progress), ("restored", *restored)] {
            let mut repl = progress.into_function_call().unwrap().into_repl();
            repl.feed_run(
                "assert state['events'] == checkpoint11\nstate = token = checkpoint11 = None\npulse = []\npulse = None",
                vec![],
                PrintWriter::Stdout,
            )
            .unwrap();
            assert_eq!(repl.heap_entry_count(), baseline, "{stage}/{kind}: before teardown");
        }
    }
}

/// Cancellation drops a queued acquisition owner while keeping live deferred handles.
#[test]
fn combined11_counted_cancel_and_cleanup_owners() {
    let library = include_str!("../../../tools/coroutines/combined11_guest.py");
    let driver = include_str!("../../../tools/coroutines/combined11_driver.py");
    let helpers = include_str!("../../../tools/coroutines/combined11_ownership.py");
    let output = checked_run(&format!(
        "{library}\n{driver}\n{helpers}\n{}",
        r"
def adapt(body):
    return body
token = []
state = new_driver('normal')
start11()
settle11('T1', token)
cancel11()
step11()
step11('T1')
step11('T1')
wait = state['tasks']['T1']['wait']
assert wait['phase'] == 'resource-cleanup'
assert wait['payload'][1] is token
settle11('T1', token)
cancel11('repeat')
assert state['tasks']['T1']['resume'][1] is token
step11('T1')
settle11('T1')
step11('T1')
retire11('T2')
step11()
settle11()
step11()
assert state['tasks']['T0']['result']['kind'] == 'cancelled'
state = wait = None
None
"
    ));
    assert_eq!(output.counts["token"], 1, "only the global token remains");
}

/// R11-1: release protected nested-exit graphs while globals and heap remain live.
#[test]
fn combined12_nested_cancel_abandon_before_teardown() {
    let library = include_str!("../../../tools/coroutines/combined11_guest.py").replace(
        "inner.defer('inner-arbitrary', 'I')",
        "inner.defer('inner-arbitrary', token)",
    );
    let driver = include_str!("../../../tools/coroutines/combined11_driver.py");
    let helpers = include_str!("../../../tools/coroutines/combined11_ownership.py");
    for (stage, cut) in [
        ("inner-drain", "cancel11('LATE-INNER')"),
        ("inner-cleanup", "cancel11('LATE-INNER')\nstep11()"),
        (
            "queued-inner-value",
            "step11()\nsettle11(value=token)\ncancel11('LATE-INNER')\nassert state['tasks']['T0']['cancel_pending']\nassert state['tasks']['T0']['resume'][1] is token",
        ),
        (
            "queued-inner-error",
            "step11()\nsettle11(value=token, outcome='error')\ncancel11('LATE-INNER')\nassert state['tasks']['T0']['resume'][1] is token",
        ),
        (
            "queued-body-cancel",
            "step11()\nsettle11(value=token)\ncancel11('LATE-INNER')\nstep11()\nassert state['tasks']['T0']['resume'] == ['error', 'LATE-INNER']\nassert not state['tasks']['T0']['cancel_pending']\nassert state['scopes']['S1']['phase'] == 'exited'",
        ),
        (
            "outer-drain",
            "step11()\nsettle11(value=token)\ncancel11('LATE-INNER')\nstep11()\nstep11()",
        ),
        (
            "outer-cleanup",
            "step11()\nsettle11(value=token)\ncancel11('LATE-INNER')\nstep11()\nstep11()\nstep11()",
        ),
        (
            "queued-outer-reply",
            "step11()\nsettle11(value=token)\ncancel11('LATE-INNER')\nstep11()\nstep11()\nstep11()\nsettle11(value=token)\ncancel11('repeat')",
        ),
    ] {
        let mut repl = new_repl();
        repl.feed_run(
            &format!(
                "{library}\n{driver}\n{helpers}\ndef adapt(body):\n    return body\nstate = token = checkpoint12 = None"
            ),
            vec![("host".to_owned(), MontyObject::function("host", None))],
            PrintWriter::Stdout,
        )
        .unwrap();
        let baseline = repl.heap_entry_count();
        let source = format!(
            "token = []\ntoken.append(token)\nstate = new_driver('nested')\nstep11()\nsettle11(value=token)\nstep11()\nassert state['scopes']['S1']['deferred'][0][1] is token\n{cut}\ncheckpoint12 = state['events'][:]\nhost()"
        );
        let progress = repl.feed_start(&source, vec![], PrintWriter::Stdout).unwrap();
        let bytes = dump("host.py", None, &[], SessionRef::Suspended(&progress)).unwrap();
        let Session::Suspended(restored) = Dump::load(&bytes).unwrap().state else {
            panic!("suspended")
        };
        for (kind, progress) in [("original", progress), ("restored", *restored)] {
            let mut repl = progress.into_function_call().unwrap().into_repl();
            repl.feed_run(
                "assert state['events'] == checkpoint12\nif state['scopes']['S1']['phase'] != 'exited':\n    assert state['scopes']['S1']['deferred'][0][1] is token\n    if state['tasks']['T0']['wait'] is None and state['tasks']['T0']['phase'] == 'resource-cleanup':\n        assert state['tasks']['T0']['resume'][1] is token\n        assert state['tasks']['T0']['cancel_pending']\nstate = token = checkpoint12 = None\npulse = []\npulse = None",
                vec![],
                PrintWriter::Stdout,
            )
            .unwrap();
            assert_eq!(repl.heap_entry_count(), baseline, "{stage}/{kind}: before teardown");
        }
    }
}

/// Accepted cleanup aliases survive deferred cancellation and return to one owner.
#[test]
fn combined12_nested_cancel_alias_owners() {
    let library = include_str!("../../../tools/coroutines/combined11_guest.py").replace(
        "inner.defer('inner-arbitrary', 'I')",
        "inner.defer('inner-arbitrary', token)",
    );
    let driver = include_str!("../../../tools/coroutines/combined11_driver.py");
    let helpers = include_str!("../../../tools/coroutines/combined11_ownership.py");
    let output = checked_run(&format!(
        "{library}\n{driver}\n{helpers}\n{}",
        r"
def adapt(body):
    return body
token = []
state = new_driver('nested')
step11()
settle11(value=token)
step11()
step11()
assert state['tasks']['T0']['wait']['payload'][1] is token
settle11(value=token)
cancel11('LATE-INNER')
assert state['tasks']['T0']['cancel_pending']
assert state['tasks']['T0']['resume'][1] is token
cancel11('repeat')
assert state['tasks']['T0']['resume'][1] is token
step11()
assert state['scopes']['S1']['cursor'] == 0
assert state['scopes']['S1']['failures'] == []
assert state['tasks']['T0']['resume'] == ['error', 'LATE-INNER']
step11()
step11()
assert state['tasks']['T0']['wait']['payload'] == ['parent-arbitrary', 'P']
settle11()
step11()
assert state['tasks']['T0']['result'] == {'kind': 'cancelled', 'value': None, 'cause': 'LATE-INNER', 'cancel': 'LATE-INNER', 'failures': [], 'children': []}
state = None
None
"
    ));
    assert_eq!(output.counts["token"], 1, "only the global token remains");
}

/// R12-1: new contexts and rejected input owners release on original/restored abandonment.
#[test]
fn combined13_fresh_scope_abandon_before_teardown() {
    let library = include_str!("../../../tools/coroutines/combined11_guest.py");
    let driver = include_str!("../../../tools/coroutines/combined11_driver.py");
    let helpers = include_str!("../../../tools/coroutines/combined11_ownership.py");
    let fixture = include_str!("../../../tools/coroutines/combined13_ownership.py");
    for (stage, cut) in [
        ("new-scope", "prepare13()"),
        ("queued-rejection", "prepare13()\nhost()"),
        ("host-rejected-repeat", "prepare13()\nreject13()\nhost()"),
        ("fresh-body", "prepare13()\nreject13()\nstep11()\nhost()"),
        ("fresh-drain", "prepare13()\nstep11()\nsettle11()\nstep11()\nhost()"),
        (
            "fresh-cleanup",
            "prepare13()\nstep11()\nsettle11()\nstep11()\nstep11()\nhost()",
        ),
        (
            "fresh-queued-reply",
            "prepare13()\nstep11()\nsettle11()\nstep11()\nstep11()\nsettle11(value=token)\ncancel11('REPEAT-CLEANUP')\nhost()",
        ),
        (
            "outer-cleanup",
            "prepare13()\nstep11()\nsettle11()\nstep11()\nstep11()\nsettle11(value=token)\nstep11()\nstep11()\nhost()",
        ),
    ] {
        let mut repl = new_repl();
        repl.feed_run(
            &format!("{library}\n{driver}\n{helpers}\n{fixture}\ndef adapt(body):\n    return body\nstate = token = checkpoint13 = cut13 = None"),
            vec![("host".to_owned(), MontyObject::function("host", None))],
            PrintWriter::Stdout,
        ).unwrap();
        let baseline = repl.heap_entry_count();
        let source = format!(
            "token = []\ntoken.append(token)\nstate = new_driver('normal')\ncut13 = '{stage}'\n{}",
            cut.replace("host()", "checkpoint13 = state['events'][:]\nhost()")
        );
        let progress = repl.feed_start(&source, vec![], PrintWriter::Stdout).unwrap();
        let bytes = dump("host.py", None, &[], SessionRef::Suspended(&progress)).unwrap();
        let Session::Suspended(restored) = Dump::load(&bytes).unwrap().state else {
            panic!("suspended")
        };
        for (kind, progress) in [("original", progress), ("restored", *restored)] {
            let mut repl = progress.into_function_call().unwrap().into_repl();
            repl.feed_run(
                "assert state['events'] == checkpoint13\nassert state['next_task'] == 1\nassert len(state['tasks']) == 1\nif state['scopes']['S2']['phase'] != 'exited':\n    assert state['scopes']['S2']['deferred'][0][1] is token\nassert state['scopes']['S0']['deferred'][0][1] is token\nstate = token = checkpoint13 = cut13 = None\npulse = []\npulse = None",
                vec![], PrintWriter::Stdout,
            ).unwrap();
            assert_eq!(repl.heap_entry_count(), baseline, "{stage}/{kind}: before teardown");
        }
    }
}

/// Rejection releases label/mode inputs while deferred handles survive real cleanup.
#[test]
fn combined13_rejected_spawn_and_deferred_alias_owners() {
    let library = include_str!("../../../tools/coroutines/combined11_guest.py");
    let driver = include_str!("../../../tools/coroutines/combined11_driver.py");
    let helpers = include_str!("../../../tools/coroutines/combined11_ownership.py");
    let fixture = include_str!("../../../tools/coroutines/combined13_ownership.py");
    let output = checked_run(&format!(
        "{library}\n{driver}\n{helpers}\n{fixture}\n{}",
        r"
def adapt(body):
    return body
cut13 = None
token = []
state = new_driver('normal')
prepare13()
reject13()
step11()
settle11()
step11()
step11()
assert state['tasks']['T0']['wait']['payload'][1] is token
settle11(value=token)
cancel11('REPEAT-CLEANUP')
assert state['tasks']['T0']['resume'][1] is token
step11()
step11()
assert state['tasks']['T0']['wait']['payload'][1] is token
settle11(value=token)
step11()
assert state['tasks']['T0']['result'] == {'kind':'cancelled','value':'returned','cause':'FIRST','cancel':'FIRST','failures':[],'children':[]}
assert state['scopes']['S2']['deferred'] == []
state = None
None
"
    ));
    assert_eq!(output.counts["token"], 1, "only the intentional global owner remains");
}

/// Coordinator census while the heap and globals remain live, including independent restores.
#[test]
fn coordinator1_live_heap_and_abandonment() {
    let source = [
        include_str!("../../../tools/coroutines/coordinator1/bindings.py"),
        include_str!("../../../tools/coroutines/coordinator1/kernel.py"),
        include_str!("../../../tools/coroutines/coordinator1/coordinator.py"),
        include_str!("../../../tools/coroutines/coordinator1/stress_workload.py"),
        include_str!("../../../tools/coroutines/coordinator1/ownership.py"),
        "def adapt(body):\n    return body\nstate = None",
    ]
    .join("\n");
    for (stage, prepare) in [
        ("ready", "state = new_driver('success', False)\nturn(state, [], 4)"),
        ("sample", "state = new_driver('success', False)\npump(state, [])"),
        (
            "cleanup",
            "state = new_driver('cancel', False)\npump(state, [])\nfor _ in range(20):\n    settle_stress()\ncancel_stress()\nassert any(t['wait'] is not None and t['wait']['payload'][0] == 'audit' for t in state['tasks'].values())",
        ),
    ] {
        let mut repl = new_repl();
        repl.feed_run(
            &source,
            vec![("host".to_owned(), MontyObject::function("host", None))],
            PrintWriter::Stdout,
        )
        .unwrap();
        let baseline = repl.heap_entry_count();
        let progress = repl
            .feed_start(&format!("{prepare}\nhost()"), vec![], PrintWriter::Stdout)
            .unwrap();
        let bytes = dump("host.py", None, &[], SessionRef::Suspended(&progress)).unwrap();
        let Session::Suspended(restored) = Dump::load(&bytes).unwrap().state else {
            panic!("suspended")
        };
        for (kind, progress) in [("original", progress), ("restored", *restored)] {
            let mut repl = progress.into_function_call().unwrap().into_repl();
            if stage == "sample" {
                let mut counts = Vec::new();
                for _ in 0..28 {
                    repl.feed_run("settle_stress()", vec![], PrintWriter::Stdout).unwrap();
                    counts.push(repl.heap_entry_count());
                }
                let low = *counts.iter().min().unwrap();
                let high = *counts.iter().max().unwrap();
                eprintln!("coordinator1/{kind}: live entries at cycles 100..1450: {counts:?}");
                assert!(
                    high - low <= 8 && high <= baseline + 160,
                    "coordinator live ownership grew: {low}..{high}"
                );
            }
            repl.feed_run("state = None\npulse = []\npulse = None", vec![], PrintWriter::Stdout)
                .unwrap();
            eprintln!(
                "coordinator1/{stage}/{kind}: baseline={baseline}, abandoned={}",
                repl.heap_entry_count()
            );
            assert_eq!(repl.heap_entry_count(), baseline, "{stage}/{kind}: before teardown");
        }
    }
}

/// Native1 keeps host-dialect generators on existing Scheduler task frames.
/// Both original and loaded graphs reclaim aliases/cycles before heap teardown.
#[test]
fn native1_scheduler_cleanup_ownership() {
    let definitions = include_str!("../../../tools/coroutines/native1/semantics.py");
    let mut repl = new_repl();
    repl.feed_run(
        definitions,
        vec![("exchange".into(), MontyObject::function("exchange", None))],
        PrintWriter::Stdout,
    )
    .unwrap();
    let baseline = repl.heap_entry_count();
    let mut progress = repl
        .feed_start("semantic_body()._native1()", vec![], PrintWriter::Stdout)
        .unwrap();
    for phase in ["read", "audit", "close"] {
        let call = progress.into_function_call().unwrap();
        let observation: serde_json::Value = serde_json::from_str(call.args.arg(0).unwrap().as_str().unwrap()).unwrap();
        assert_eq!(observation["phase"], phase);
        progress = call.resume_pending(PrintWriter::Stdout).unwrap();
        assert!(matches!(progress, ReplProgress::ResolveFutures(_)));
        let bytes = dump("native1.py", None, &[], SessionRef::Suspended(&progress)).unwrap();
        let Session::Suspended(restored) = Dump::load(&bytes).unwrap().state else {
            panic!("native pending future")
        };
        // No guest cleanup is run on either independent physical abandonment.
        for (kind, pending) in [("original", progress), ("restored", *restored)] {
            let mut repl = pending.into_repl();
            let expected = match phase {
                "read" => "[]",
                "audit" => "[['caught', 'task-cancel:FIRST']]",
                _ => "[['caught', 'task-cancel:FIRST'], ['audit-error', 'AUDIT']]",
            };
            repl.feed_run(
                &format!("assert TRACE == {expected}\nTRACE = []\npulse = []\npulse = None"),
                vec![],
                PrintWriter::Stdout,
            )
            .unwrap();
            eprintln!(
                "native1/{phase}/{kind}: baseline={baseline}, abandoned={}",
                repl.heap_entry_count()
            );
            assert_eq!(
                repl.heap_entry_count(),
                baseline,
                "native frames/aliases/cycle reclaimed before teardown"
            );
        }
        let Session::Suspended(restored) = Dump::load(&bytes).unwrap().state else {
            panic!("native pending future")
        };
        let pending = restored.into_resolve_futures().unwrap();
        let id = pending.pending_call_ids()[0];
        let answer = match phase {
            "read" => ExtFunctionResult::Error(MontyException::new(
                ExcType::ValueError,
                Some("task-cancel:FIRST".into()),
            )),
            "audit" => ExtFunctionResult::Error(MontyException::new(ExcType::ValueError, Some("AUDIT".into()))),
            _ => ExtFunctionResult::Return(MontyObject::string("\"closed\"")),
        };
        progress = pending.resume(vec![(id, answer)], PrintWriter::Stdout).unwrap();
    }
    let (mut repl, value) = progress.into_complete().unwrap();
    assert_eq!(value.as_ref().as_str(), Some("caught"));
    repl.feed_run("assert TRACE == [['caught', 'task-cancel:FIRST'], ['audit-error', 'AUDIT'], ['close', 'closed']]\nTRACE = []\npulse = []\npulse = None", vec![], PrintWriter::Stdout).unwrap();
    assert_eq!(repl.heap_entry_count(), baseline);
}

/// Opt-in yields must not change ordinary send, delegation or asyncio await.
#[test]
fn native1_entry_is_explicit() {
    checked_run(
        r#"
import asyncio
def ordinary():
    yield 7
    return 9
g = ordinary()
assert next(g) == 7
try:
    g.send(None)
except StopIteration as done:
    assert done.value == 9
async def invalid():
    await ordinary()
try:
    asyncio.run(invalid())
except TypeError as error:
    assert str(error) == "'generator' object can't be awaited"
else:
    assert False, 'ordinary generator await accepted'
def bad():
    try:
        yield 7
    except TypeError:
        return 'bad-yield'
assert bad()._native1() == 'bad-yield'
g = ordinary = invalid = bad = None
None
"#,
    );
}

/// Distinct upstream tasks own native generator chains; reversed host results
/// wake the right chain without a Python ready queue or trampoline.
#[test]
fn native1_composes_with_existing_gather_tasks() {
    let mut repl = new_repl();
    repl.feed_run(
        "from __monty_host__ import host_coroutines\ndef request(label):\n    return (yield exchange(label))\nasync def body(label):\n    return await request(label)",
        vec![("exchange".into(), MontyObject::function("exchange", None))],
        PrintWriter::Stdout,
    ).unwrap();
    // This feed has ordinary async semantics; the host-mode bodies above keep
    // their compiled identity. Both share the same intern table and heap.
    repl.feed_run(
        "import asyncio\nasync def wrapper(label):\n    return body(label)._native1()",
        vec![],
        PrintWriter::Stdout,
    )
    .unwrap();
    let baseline = repl.heap_entry_count();
    let mut progress = repl
        .feed_start(
            "await asyncio.gather(wrapper('A'), wrapper('B'))",
            vec![],
            PrintWriter::Stdout,
        )
        .unwrap();
    let mut ids = Vec::new();
    while let ReplProgress::FunctionCall(call) = progress {
        ids.push(call.call_id);
        progress = call.resume_pending(PrintWriter::Stdout).unwrap();
    }
    assert_eq!(ids.len(), 2);
    let bytes = dump("native1.py", None, &[], SessionRef::Suspended(&progress)).unwrap();
    let Session::Suspended(restored) = Dump::load(&bytes).unwrap().state else {
        panic!("tasks")
    };
    for pending in [progress, *restored] {
        let mut repl = pending.into_repl();
        repl.feed_run("pulse = []\npulse = None", vec![], PrintWriter::Stdout)
            .unwrap();
        assert_eq!(repl.heap_entry_count(), baseline);
    }
    let Session::Suspended(restored) = Dump::load(&bytes).unwrap().state else {
        panic!("tasks")
    };
    let (mut repl, value) = restored
        .into_resolve_futures()
        .unwrap()
        .resume(
            vec![
                (ids[1], ExtFunctionResult::Return(MontyObject::string("b"))),
                (ids[0], ExtFunctionResult::Return(MontyObject::string("a"))),
            ],
            PrintWriter::Stdout,
        )
        .unwrap()
        .into_complete()
        .unwrap();
    assert_eq!(
        value,
        MontyObject::list([MontyObject::string("a"), MontyObject::string("b")])
    );
    repl.feed_run("pulse = []\npulse = None", vec![], PrintWriter::Stdout)
        .unwrap();
    assert_eq!(repl.heap_entry_count(), baseline);
}

/// Native cancellation metadata owns no heap references. The cancelled task still
/// owns its live frames, aliases and cycles until completion or physical disposal.
#[test]
fn native2_cancelled_cleanup_live_ownership() {
    let mut repl = new_repl();
    repl.feed_run(
        include_str!("../../../tools/coroutines/cancelled_error/migrated/native1/cancellation_ownership.py"),
        vec![("exchange".into(), MontyObject::function("exchange", None))],
        PrintWriter::Stdout,
    )
    .unwrap();
    let baseline = repl.heap_entry_count();
    let mut progress = repl
        .feed_start("cancellation_body()._native1()", vec![], PrintWriter::Stdout)
        .unwrap();
    for (phase, trace) in [
        ("body", "[]"),
        ("inner", "[['caught', ''], 'inner']"),
        ("outer", "[['caught', ''], 'inner', 'outer']"),
    ] {
        let call = progress.into_function_call().unwrap();
        let observation: serde_json::Value = serde_json::from_str(call.args.arg(0).unwrap().as_str().unwrap()).unwrap();
        assert_eq!(observation["phase"], phase);
        progress = call.resume_pending(PrintWriter::Stdout).unwrap();
        let bytes = dump("native2.py", None, &[], SessionRef::Suspended(&progress)).unwrap();
        let Session::Suspended(copy) = Dump::load(&bytes).unwrap().state else {
            panic!("pending")
        };
        for (kind, pending) in [("original", progress), ("restored", *copy)] {
            let mut repl = pending.into_repl();
            repl.feed_run(
                &format!("assert TRACE == {trace}\nTRACE = []\npulse = []\npulse = None"),
                vec![],
                PrintWriter::Stdout,
            )
            .unwrap();
            eprintln!(
                "native2/{phase}/{kind}: baseline={baseline}, abandoned={}",
                repl.heap_entry_count()
            );
            assert_eq!(repl.heap_entry_count(), baseline, "live census before teardown");
        }
        let Session::Suspended(copy) = Dump::load(&bytes).unwrap().state else {
            panic!("pending")
        };
        let pending = copy.into_resolve_futures().unwrap();
        let id = pending.pending_call_ids()[0];
        let results = if phase == "body" {
            vec![]
        } else {
            vec![(id, ExtFunctionResult::Return(MontyObject::string("\"ok\"")))]
        };
        progress = pending
            .resume_native(
                results,
                Some(if phase == "body" { "" } else { "REPEAT" }.into()),
                PrintWriter::Stdout,
            )
            .unwrap();
    }
    let (mut repl, value) = progress.into_complete().unwrap();
    assert_eq!(
        value,
        MontyObject::dict([
            (MontyObject::string("status"), MontyObject::string("cancelled")),
            (MontyObject::string("reason"), MontyObject::string("")),
            (MontyObject::string("value"), MontyObject::string("caught")),
            (MontyObject::string("error"), MontyObject::none()),
        ])
    );
    repl.feed_run(
        "assert TRACE == [['caught', ''], 'inner', 'outer']\nTRACE = []\npulse = []\npulse = None",
        vec![],
        PrintWriter::Stdout,
    )
    .unwrap();
    assert_eq!(
        repl.heap_entry_count(),
        baseline,
        "cancelled completion before teardown"
    );
}

/// A host-cancel request cannot opt an ordinary asyncio task into native policy.
#[test]
fn native2_rejects_default_await_and_preserves_guest_errors() {
    let repl = new_repl();
    let progress = repl
        .feed_start(
            "try:\n    await exchange()\nexcept ValueError as e:\n    result = str(e)\nresult",
            vec![("exchange".into(), MontyObject::function("exchange", None))],
            PrintWriter::Stdout,
        )
        .unwrap()
        .into_function_call()
        .unwrap()
        .resume_pending(PrintWriter::Stdout)
        .unwrap();
    let bytes = dump("default.py", None, &[], SessionRef::Suspended(&progress)).unwrap();
    let pending = progress.into_resolve_futures().unwrap();
    let error = pending
        .resume_native(vec![], Some("host".into()), PrintWriter::Stdout)
        .unwrap_err();
    assert_eq!(error.error.exc_type(), ExcType::RuntimeError);
    let Session::Suspended(copy) = Dump::load(&bytes).unwrap().state else {
        panic!("pending")
    };
    let pending = copy.into_resolve_futures().unwrap();
    let id = pending.pending_call_ids()[0];
    let result = pending
        .resume(
            vec![(
                id,
                ExtFunctionResult::Error(MontyException::new(
                    ExcType::ValueError,
                    Some("task-cancel:guest".into()),
                )),
            )],
            PrintWriter::Stdout,
        )
        .unwrap();
    let (_, value) = result.into_complete().unwrap();
    assert_eq!(value, MontyObject::string("task-cancel:guest"));
}

/// Cancellation at cleanup exit releases aliased pending results and original
/// error owners through the existing unwind, before the heap itself is dropped.
#[test]
fn native4_cleanup_exit_pending_owner_census() {
    for mode in ["return", "error", "cleanup_error"] {
        let mut repl = new_repl();
        repl.feed_run(
            include_str!("../../../tools/coroutines/cancelled_error/migrated/native1/cleanup_exit_ownership.py"),
            vec![("exchange".into(), MontyObject::function("exchange", None))],
            PrintWriter::Stdout,
        )
        .unwrap();
        let baseline = repl.heap_entry_count();
        let mut progress = repl
            .feed_start(
                &format!("cleanup_exit_entry('{mode}')._native1()"),
                vec![],
                PrintWriter::Stdout,
            )
            .unwrap();
        for (phase, trace) in [("inner", "['inner']"), ("outer", "['inner', 'outer']")] {
            let call = progress.into_function_call().unwrap();
            let observation: serde_json::Value =
                serde_json::from_str(call.args.arg(0).unwrap().as_str().unwrap()).unwrap();
            assert_eq!(observation["phase"], phase);
            progress = call.resume_pending(PrintWriter::Stdout).unwrap();
            let bytes = dump("native4.py", None, &[], SessionRef::Suspended(&progress)).unwrap();
            let Session::Suspended(copy) = Dump::load(&bytes).unwrap().state else {
                panic!("pending")
            };
            for pending in [progress, *copy] {
                let mut repl = pending.into_repl();
                repl.feed_run(
                    &format!("assert TRACE == {trace}\nTRACE = []\npulse = []\npulse = None"),
                    vec![],
                    PrintWriter::Stdout,
                )
                .unwrap();
                assert_eq!(
                    repl.heap_entry_count(),
                    baseline,
                    "{mode}/{phase}: disposal before teardown"
                );
            }
            let Session::Suspended(copy) = Dump::load(&bytes).unwrap().state else {
                panic!("pending")
            };
            let pending = copy.into_resolve_futures().unwrap();
            let id = pending.pending_call_ids()[0];
            let reply = if mode == "cleanup_error" && phase == "inner" {
                ExtFunctionResult::Error(MontyException::new(ExcType::TypeError, Some("CLEANUP".into())))
            } else {
                ExtFunctionResult::Return(MontyObject::string("null"))
            };
            progress = pending
                .resume_native(
                    vec![(id, reply)],
                    Some(if phase == "inner" { "" } else { "REPEAT" }.into()),
                    PrintWriter::Stdout,
                )
                .unwrap();
        }
        let (mut repl, value) = progress.into_complete().unwrap();
        let expected_error = match mode {
            "error" => Some("TypeError: ORIGINAL"),
            "cleanup_error" => Some("TypeError: CLEANUP"),
            _ => None,
        };
        // Public export has no interpreter heap edges; inspect before dropping it.
        let rendered = format!("{value:?}");
        assert!(rendered.contains("cancelled"), "{rendered}");
        if let Some(error) = expected_error {
            assert!(rendered.contains(error), "{rendered}");
        }
        drop(value);
        let trace = if mode == "return" {
            "['inner', 'outer', ['caught', '']]"
        } else {
            "['inner', 'outer']"
        };
        repl.feed_run(
            &format!("assert TRACE == {trace}\nTRACE = []\npulse = []\npulse = None"),
            vec![],
            PrintWriter::Stdout,
        )
        .unwrap();
        eprintln!(
            "native4/{mode}: baseline={baseline}, completed={}",
            repl.heap_entry_count()
        );
        assert_eq!(repl.heap_entry_count(), baseline, "{mode}: terminal before teardown");
    }
}

/// Bypassed dispatch releases error aliases, cycles and an active handler binding.
#[test]
fn native5_dispatch_error_owner_census() {
    for mode in ["error", "cleanup_error"] {
        let mut repl = new_repl();
        repl.feed_run(
            r"from __monty_host__ import host_coroutines
import json
TRACE = []
def operation(name):
    return json.loads((yield exchange(json.dumps({'phase': name}))))
def choose():
    TRACE.append('FORBIDDEN-match')
    return TypeError
async def cleanup_exit_entry(mode):
    token = []
    token.append(token)
    failure = TypeError('ORIGINAL')
    aliases = [token, token, failure, failure]
    try:
        try:
            raise ValueError('bound')
        except ValueError as bound:
            try:
                raise failure
            finally:
                TRACE.append('inner')
                await operation('inner')
    except choose() as error:
        TRACE.append('FORBIDDEN-handler')
        raise
    finally:
        assert aliases[0] is token and aliases[1] is token
        assert aliases[2] is failure and aliases[3] is failure
        TRACE.append('outer')
        await operation('outer')
",
            vec![("exchange".into(), MontyObject::function("exchange", None))],
            PrintWriter::Stdout,
        )
        .unwrap();
        let baseline = repl.heap_entry_count();
        let mut progress = repl
            .feed_start(
                &format!("cleanup_exit_entry('{mode}')._native1()"),
                vec![],
                PrintWriter::Stdout,
            )
            .unwrap();
        for (phase, trace) in [("inner", "['inner']"), ("outer", "['inner', 'outer']")] {
            let call = progress.into_function_call().unwrap();
            let observation: serde_json::Value =
                serde_json::from_str(call.args.arg(0).unwrap().as_str().unwrap()).unwrap();
            assert_eq!(observation["phase"], phase);
            progress = call.resume_pending(PrintWriter::Stdout).unwrap();
            let bytes = dump("native5.py", None, &[], SessionRef::Suspended(&progress)).unwrap();
            let Session::Suspended(copy) = Dump::load(&bytes).unwrap().state else {
                panic!("pending")
            };
            for pending in [progress, *copy] {
                let mut repl = pending.into_repl();
                repl.feed_run(
                    &format!("assert TRACE == {trace}\nTRACE = []\npulse = []\npulse = None"),
                    vec![],
                    PrintWriter::Stdout,
                )
                .unwrap();
                assert_eq!(
                    repl.heap_entry_count(),
                    baseline,
                    "{mode}/{phase}: disposal before teardown"
                );
            }
            let Session::Suspended(copy) = Dump::load(&bytes).unwrap().state else {
                panic!("pending")
            };
            let pending = copy.into_resolve_futures().unwrap();
            let id = pending.pending_call_ids()[0];
            let reply = if mode == "cleanup_error" && phase == "inner" {
                ExtFunctionResult::Error(MontyException::new(ExcType::TypeError, Some("CLEANUP".into())))
            } else {
                ExtFunctionResult::Return(MontyObject::string("null"))
            };
            progress = pending
                .resume_native(
                    vec![(id, reply)],
                    Some(if phase == "inner" { "" } else { "REPEAT" }.into()),
                    PrintWriter::Stdout,
                )
                .unwrap();
        }
        let (mut repl, value) = progress.into_complete().unwrap();
        drop(value);
        let trace = if mode == "return" {
            "['inner', 'outer', ['caught', '']]"
        } else {
            "['inner', 'outer']"
        };
        repl.feed_run(
            &format!("assert TRACE == {trace}\nTRACE = []\npulse = []\npulse = None"),
            vec![],
            PrintWriter::Stdout,
        )
        .unwrap();
        eprintln!(
            "native5/{mode}: baseline={baseline}, completed={}",
            repl.heap_entry_count()
        );
        assert_eq!(repl.heap_entry_count(), baseline, "{mode}: terminal before teardown");
    }
}

/// Active matching keeps original identity across independent restore/disposal cuts.
#[test]
fn native6_active_dispatch_owner_census() {
    for (mode, restoring) in [
        ("explicit", false),
        ("explicit", true),
        ("implicit", false),
        ("implicit", true),
        ("cleanup_error", false),
        ("cleanup_error", true),
    ] {
        let mut repl = new_repl();
        repl.feed_run(
            r"from __monty_host__ import host_coroutines
import json
TRACE = []
def operation(name):
    return json.loads((yield exchange(json.dumps({'phase': name}))))
async def choose(mode):
    try:
        if mode != 'implicit':
            return ValueError
    finally:
        TRACE.append('inner')
        await operation('inner')
async def cleanup_exit_entry(mode):
    token = []
    token.append(token)
    failure = TypeError('ORIGINAL')
    aliases = [token, token, failure, failure]
    try:
        try:
            raise ValueError('bound')
        except ValueError as bound:
            try:
                raise failure
            except (await choose(mode)) or ValueError as selected:
                TRACE.append('FORBIDDEN-handler')
    finally:
        try:
            raise
        except TypeError as active:
            if mode != 'cleanup_error':
                assert active is failure and active is aliases[2]
            else:
                assert str(active) == 'CLEANUP'
        assert aliases[0] is token and aliases[1] is token
        assert aliases[2] is failure and aliases[3] is failure
        TRACE.append('outer')
        await operation('outer')
",
            vec![("exchange".into(), MontyObject::function("exchange", None))],
            PrintWriter::Stdout,
        )
        .unwrap();
        let baseline = repl.heap_entry_count();
        let mut progress = repl
            .feed_start(
                &format!("cleanup_exit_entry('{mode}')._native1()"),
                vec![],
                PrintWriter::Stdout,
            )
            .unwrap();
        for (phase, trace) in [("inner", "['inner']"), ("outer", "['inner', 'outer']")] {
            let call = progress.into_function_call().unwrap();
            let observation: serde_json::Value =
                serde_json::from_str(call.args.arg(0).unwrap().as_str().unwrap()).unwrap();
            assert_eq!(observation["phase"], phase);
            progress = call.resume_pending(PrintWriter::Stdout).unwrap();
            let bytes = dump("native6.py", None, &[], SessionRef::Suspended(&progress)).unwrap();
            let Session::Suspended(copy) = Dump::load(&bytes).unwrap().state else {
                panic!("pending")
            };
            let (disposed, continuation) = if restoring {
                let Session::Suspended(continuation) = Dump::load(&bytes).unwrap().state else {
                    panic!("pending")
                };
                (vec![progress, *copy], *continuation)
            } else {
                (vec![*copy], progress)
            };
            for pending in disposed {
                let mut repl = pending.into_repl();
                repl.feed_run(
                    &format!("assert TRACE == {trace}\nTRACE = []\npulse = []\npulse = None"),
                    vec![],
                    PrintWriter::Stdout,
                )
                .unwrap();
                assert_eq!(
                    repl.heap_entry_count(),
                    baseline,
                    "{mode}/{phase}: disposal before teardown"
                );
            }
            let pending = continuation.into_resolve_futures().unwrap();
            let id = pending.pending_call_ids()[0];
            let reply = if mode == "cleanup_error" && phase == "inner" {
                ExtFunctionResult::Error(MontyException::new(ExcType::TypeError, Some("CLEANUP".into())))
            } else {
                ExtFunctionResult::Return(MontyObject::string("null"))
            };
            progress = pending
                .resume_native(
                    vec![(id, reply)],
                    Some(if phase == "inner" { "" } else { "REPEAT" }.into()),
                    PrintWriter::Stdout,
                )
                .unwrap();
        }
        let (mut repl, value) = progress.into_complete().unwrap();
        drop(value);
        let trace = "['inner', 'outer']";
        repl.feed_run(
            &format!("assert TRACE == {trace}\nTRACE = []\npulse = []\npulse = None"),
            vec![],
            PrintWriter::Stdout,
        )
        .unwrap();
        eprintln!(
            "native6/{mode}/restoring={restoring}: baseline={baseline}, completed={}",
            repl.heap_entry_count()
        );
        assert_eq!(repl.heap_entry_count(), baseline, "{mode}: terminal before teardown");
    }
}

/// Policy7 metadata must not retain cyclic args, pending returns, or caught errors.
#[test]
fn native7_catchable_diagnostic_live_census() {
    for mode in ["return", "unsupported", "partial"] {
        for restoring in [false, true] {
            let mut repl = new_repl();
            repl.feed_run(
                r"from __monty_host__ import host_coroutines
import asyncio
TRACE = []
def operation(name):
    return (yield exchange(name))
async def policy7(mode):
    token = []
    token.append(token)
    original = TypeError(token)
    alias = original
    try:
        try:
            raise original
        finally:
            assert original is alias
            await operation('cleanup')
            assert original is alias
    except asyncio.CancelledError as signal:
        assert str(signal) == ''
        await operation('recovery')
        if mode == 'unsupported':
            return TypeError(token)
        if mode == 'partial':
            return ['prefix', token, TypeError(token)]
        return 'recovered'
",
                vec![("exchange".into(), MontyObject::function("exchange", None))],
                PrintWriter::Stdout,
            )
            .unwrap();
            let baseline = repl.heap_entry_count();
            let mut progress = repl
                .feed_start(&format!("policy7('{mode}')._native1()"), vec![], PrintWriter::Stdout)
                .unwrap();
            for phase in ["cleanup", "recovery"] {
                let call = progress.into_function_call().unwrap();
                assert_eq!(call.args.arg(0).unwrap().as_str(), Some(phase));
                progress = call.resume_pending(PrintWriter::Stdout).unwrap();
                let bytes = dump("native7.py", None, &[], SessionRef::Suspended(&progress)).unwrap();
                let Session::Suspended(copy) = Dump::load(&bytes).unwrap().state else {
                    panic!("suspended")
                };
                let (disposed, continuation) = if restoring {
                    (progress, *copy)
                } else {
                    (*copy, progress)
                };
                let mut disposed = disposed.into_repl();
                disposed
                    .feed_run("pulse = []\npulse = None", vec![], PrintWriter::Stdout)
                    .unwrap();
                assert_eq!(
                    disposed.heap_entry_count(),
                    baseline,
                    "{mode}/{phase}: disposal before teardown"
                );
                let pending = continuation.into_resolve_futures().unwrap();
                let id = pending.pending_call_ids()[0];
                progress = pending
                    .resume_native(
                        vec![(id, ExtFunctionResult::Return(MontyObject::none()))],
                        Some(if phase == "cleanup" { "" } else { "REPEAT" }.into()),
                        PrintWriter::Stdout,
                    )
                    .unwrap();
            }
            let (mut repl, value) = progress.into_complete().unwrap();
            let fields = value.as_ref().pairs().unwrap();
            let field = |name| fields.iter().find(|(key, _)| key.as_str() == Some(name)).unwrap().1;
            assert_eq!(field("status").as_str(), Some("cancelled"));
            assert_eq!(field("reason").as_str(), Some(""));
            assert_eq!(
                field("value_export_error").as_str(),
                if mode == "return" {
                    None
                } else {
                    Some("unsupported-value")
                }
            );
            let diagnostic = field("interrupted_error").pairs().unwrap();
            assert!(
                diagnostic
                    .iter()
                    .any(|(key, value)| key.as_str() == Some("type") && value.as_str() == Some("TypeError"))
            );
            drop(value);
            repl.feed_run("pulse = []\npulse = None", vec![], PrintWriter::Stdout)
                .unwrap();
            eprintln!(
                "native7/{mode}/{restoring}: baseline={baseline}, completed={}",
                repl.heap_entry_count()
            );
            assert_eq!(repl.heap_entry_count(), baseline, "{mode}: terminal before teardown");
        }
    }
}

/// Genuine old code must remain on its original cancellation-contract runtime.
#[test]
fn native7_genuine_legacy_activation_and_delivered_recovery() {
    assert!(matches!(
        Dump::load(include_bytes!("fixtures/dump_v22.bin")),
        Err(monty::DumpError::VersionTooOld { .. })
    ));
}

/// Separately authored policy7 counterpart; historical assertions above stay frozen.
#[test]
fn native7_policy2_owner_census() {
    let mut repl = new_repl();
    repl.feed_run(
        include_str!("../../../tools/coroutines/cancelled_error/migrated/native1/cancellation_ownership.py"),
        vec![("exchange".into(), MontyObject::function("exchange", None))],
        PrintWriter::Stdout,
    )
    .unwrap();
    let baseline = repl.heap_entry_count();
    let mut progress = repl
        .feed_start("cancellation_body()._native1()", vec![], PrintWriter::Stdout)
        .unwrap();
    for (phase, trace) in [
        ("body", "[]"),
        ("inner", "[['caught', ''], 'inner']"),
        ("outer", "[['caught', ''], 'inner', 'outer']"),
    ] {
        let call = progress.into_function_call().unwrap();
        let observation: serde_json::Value = serde_json::from_str(call.args.arg(0).unwrap().as_str().unwrap()).unwrap();
        assert_eq!(observation["phase"], phase);
        progress = call.resume_pending(PrintWriter::Stdout).unwrap();
        let bytes = dump("policy7-native2.py", None, &[], SessionRef::Suspended(&progress)).unwrap();
        let Session::Suspended(copy) = Dump::load(&bytes).unwrap().state else {
            panic!("pending")
        };
        for (kind, pending) in [("original", progress), ("restored", *copy)] {
            let mut repl = pending.into_repl();
            repl.feed_run(
                &format!("assert TRACE == {trace}\nTRACE = []\npulse = []\npulse = None"),
                vec![],
                PrintWriter::Stdout,
            )
            .unwrap();
            eprintln!(
                "policy7-native2/{phase}/{kind}: baseline={baseline}, abandoned={}",
                repl.heap_entry_count()
            );
            assert_eq!(repl.heap_entry_count(), baseline, "live census before teardown");
        }
        let Session::Suspended(copy) = Dump::load(&bytes).unwrap().state else {
            panic!("pending")
        };
        let pending = copy.into_resolve_futures().unwrap();
        let id = pending.pending_call_ids()[0];
        let results = if phase == "body" {
            vec![]
        } else {
            vec![(id, ExtFunctionResult::Return(MontyObject::string("\"ok\"")))]
        };
        progress = pending
            .resume_native(
                results,
                Some(if phase == "body" { "" } else { "REPEAT" }.into()),
                PrintWriter::Stdout,
            )
            .unwrap();
    }
    let (mut repl, value) = progress.into_complete().unwrap();
    assert_eq!(
        value,
        MontyObject::dict([
            (MontyObject::string("status"), MontyObject::string("cancelled")),
            (MontyObject::string("reason"), MontyObject::string("")),
            (MontyObject::string("value"), MontyObject::string("caught")),
            (MontyObject::string("error"), MontyObject::none()),
            (MontyObject::string("interrupted_error"), MontyObject::none()),
            (MontyObject::string("value_export_error"), MontyObject::none()),
        ])
    );
    repl.feed_run(
        "assert TRACE == [['caught', ''], 'inner', 'outer']\nTRACE = []\npulse = []\npulse = None",
        vec![],
        PrintWriter::Stdout,
    )
    .unwrap();
    assert_eq!(
        repl.heap_entry_count(),
        baseline,
        "cancelled completion before teardown"
    );
}

/// Separately authored policy7 counterpart; historical assertions above stay frozen.
#[test]
fn native7_policy4_owner_census() {
    for mode in ["return", "error", "cleanup_error"] {
        let mut repl = new_repl();
        repl.feed_run(
            include_str!("../../../tools/coroutines/cancelled_error/migrated/native1/cleanup_exit_ownership.py"),
            vec![("exchange".into(), MontyObject::function("exchange", None))],
            PrintWriter::Stdout,
        )
        .unwrap();
        let baseline = repl.heap_entry_count();
        let mut progress = repl
            .feed_start(
                &format!("cleanup_exit_entry('{mode}')._native1()"),
                vec![],
                PrintWriter::Stdout,
            )
            .unwrap();
        for (phase, trace) in [("inner", "['inner']"), ("outer", "['inner', 'outer']")] {
            let call = progress.into_function_call().unwrap();
            let observation: serde_json::Value =
                serde_json::from_str(call.args.arg(0).unwrap().as_str().unwrap()).unwrap();
            assert_eq!(observation["phase"], phase);
            progress = call.resume_pending(PrintWriter::Stdout).unwrap();
            let bytes = dump("policy7-native4.py", None, &[], SessionRef::Suspended(&progress)).unwrap();
            let Session::Suspended(copy) = Dump::load(&bytes).unwrap().state else {
                panic!("pending")
            };
            for pending in [progress, *copy] {
                let mut repl = pending.into_repl();
                repl.feed_run(
                    &format!("assert TRACE == {trace}\nTRACE = []\npulse = []\npulse = None"),
                    vec![],
                    PrintWriter::Stdout,
                )
                .unwrap();
                assert_eq!(
                    repl.heap_entry_count(),
                    baseline,
                    "{mode}/{phase}: disposal before teardown"
                );
            }
            let Session::Suspended(copy) = Dump::load(&bytes).unwrap().state else {
                panic!("pending")
            };
            let pending = copy.into_resolve_futures().unwrap();
            let id = pending.pending_call_ids()[0];
            let reply = if mode == "cleanup_error" && phase == "inner" {
                ExtFunctionResult::Error(MontyException::new(ExcType::TypeError, Some("CLEANUP".into())))
            } else {
                ExtFunctionResult::Return(MontyObject::string("null"))
            };
            progress = pending
                .resume_native(
                    vec![(id, reply)],
                    Some(if phase == "inner" { "" } else { "REPEAT" }.into()),
                    PrintWriter::Stdout,
                )
                .unwrap();
        }
        let (mut repl, value) = progress.into_complete().unwrap();
        let fields = value.as_ref().pairs().unwrap();
        let field = |name| fields.iter().find(|(key, _)| key.as_str() == Some(name)).unwrap().1;
        assert_eq!(field("status").as_str(), Some("cancelled"));
        assert_eq!(field("reason").as_str(), Some(""));
        assert_eq!(field("error").type_name(), "NoneType");
        if mode != "return" {
            let diagnostic = field("interrupted_error").pairs().unwrap();
            let message = diagnostic
                .iter()
                .find(|(key, _)| key.as_str() == Some("message"))
                .unwrap()
                .1;
            assert_eq!(
                message.as_str(),
                Some(if mode == "error" { "ORIGINAL" } else { "CLEANUP" })
            );
        }
        drop(value);
        let trace = "['inner', 'outer', ['caught', '']]";
        repl.feed_run(
            &format!("assert TRACE == {trace}\nTRACE = []\npulse = []\npulse = None"),
            vec![],
            PrintWriter::Stdout,
        )
        .unwrap();
        eprintln!(
            "policy7-native4/{mode}: baseline={baseline}, completed={}",
            repl.heap_entry_count()
        );
        assert_eq!(repl.heap_entry_count(), baseline, "{mode}: terminal before teardown");
    }
}

/// Separately authored policy7 counterpart; historical assertions above stay frozen.
#[test]
fn native7_policy5_owner_census() {
    for mode in ["error", "cleanup_error"] {
        let mut repl = new_repl();
        repl.feed_run(
            r"from __monty_host__ import host_coroutines
import json
TRACE = []
def operation(name):
    return json.loads((yield exchange(json.dumps({'phase': name}))))
def choose():
    TRACE.append('FORBIDDEN-match')
    return TypeError
async def cleanup_exit_entry(mode):
    token = []
    token.append(token)
    failure = TypeError('ORIGINAL')
    aliases = [token, token, failure, failure]
    try:
        try:
            raise ValueError('bound')
        except ValueError as bound:
            try:
                raise failure
            finally:
                TRACE.append('inner')
                await operation('inner')
    except choose() as error:
        TRACE.append('FORBIDDEN-handler')
        raise
    finally:
        assert aliases[0] is token and aliases[1] is token
        assert aliases[2] is failure and aliases[3] is failure
        TRACE.append('outer')
        await operation('outer')
",
            vec![("exchange".into(), MontyObject::function("exchange", None))],
            PrintWriter::Stdout,
        )
        .unwrap();
        let baseline = repl.heap_entry_count();
        let mut progress = repl
            .feed_start(
                &format!("cleanup_exit_entry('{mode}')._native1()"),
                vec![],
                PrintWriter::Stdout,
            )
            .unwrap();
        for (phase, trace) in [
            ("inner", "['inner']"),
            ("outer", "['inner', 'FORBIDDEN-match', 'outer']"),
        ] {
            let call = progress.into_function_call().unwrap();
            let observation: serde_json::Value =
                serde_json::from_str(call.args.arg(0).unwrap().as_str().unwrap()).unwrap();
            assert_eq!(observation["phase"], phase);
            progress = call.resume_pending(PrintWriter::Stdout).unwrap();
            let bytes = dump("policy7-native5.py", None, &[], SessionRef::Suspended(&progress)).unwrap();
            let Session::Suspended(copy) = Dump::load(&bytes).unwrap().state else {
                panic!("pending")
            };
            for pending in [progress, *copy] {
                let mut repl = pending.into_repl();
                repl.feed_run(
                    &format!("assert TRACE == {trace}\nTRACE = []\npulse = []\npulse = None"),
                    vec![],
                    PrintWriter::Stdout,
                )
                .unwrap();
                assert_eq!(
                    repl.heap_entry_count(),
                    baseline,
                    "{mode}/{phase}: disposal before teardown"
                );
            }
            let Session::Suspended(copy) = Dump::load(&bytes).unwrap().state else {
                panic!("pending")
            };
            let pending = copy.into_resolve_futures().unwrap();
            let id = pending.pending_call_ids()[0];
            let reply = if mode == "cleanup_error" && phase == "inner" {
                ExtFunctionResult::Error(MontyException::new(ExcType::TypeError, Some("CLEANUP".into())))
            } else {
                ExtFunctionResult::Return(MontyObject::string("null"))
            };
            progress = pending
                .resume_native(
                    vec![(id, reply)],
                    Some(if phase == "inner" { "" } else { "REPEAT" }.into()),
                    PrintWriter::Stdout,
                )
                .unwrap();
        }
        let (mut repl, value) = progress.into_complete().unwrap();
        drop(value);
        let trace = "['inner', 'FORBIDDEN-match', 'outer']";
        repl.feed_run(
            &format!("assert TRACE == {trace}\nTRACE = []\npulse = []\npulse = None"),
            vec![],
            PrintWriter::Stdout,
        )
        .unwrap();
        eprintln!(
            "policy7-native5/{mode}: baseline={baseline}, completed={}",
            repl.heap_entry_count()
        );
        assert_eq!(repl.heap_entry_count(), baseline, "{mode}: terminal before teardown");
    }
}

/// Separately authored policy7 counterpart; historical assertions above stay frozen.
#[test]
fn native7_policy6_owner_census() {
    for (mode, restoring) in [
        ("explicit", false),
        ("explicit", true),
        ("implicit", false),
        ("implicit", true),
        ("cleanup_error", false),
        ("cleanup_error", true),
    ] {
        let mut repl = new_repl();
        repl.feed_run(
            r"from __monty_host__ import host_coroutines
import json
import asyncio
TRACE = []
def operation(name):
    return json.loads((yield exchange(json.dumps({'phase': name}))))
async def choose(mode):
    try:
        if mode != 'implicit':
            return ValueError
    finally:
        TRACE.append('inner')
        await operation('inner')
async def cleanup_exit_entry(mode):
    token = []
    token.append(token)
    failure = TypeError('ORIGINAL')
    aliases = [token, token, failure, failure]
    try:
        try:
            raise ValueError('bound')
        except ValueError as bound:
            try:
                raise failure
            except (await choose(mode)) or ValueError as selected:
                TRACE.append('FORBIDDEN-handler')
    finally:
        try:
            raise
        except asyncio.CancelledError as active:
            assert str(active) == ''
        assert aliases[0] is token and aliases[1] is token
        assert aliases[2] is failure and aliases[3] is failure
        TRACE.append('outer')
        await operation('outer')
",
            vec![("exchange".into(), MontyObject::function("exchange", None))],
            PrintWriter::Stdout,
        )
        .unwrap();
        let baseline = repl.heap_entry_count();
        let mut progress = repl
            .feed_start(
                &format!("cleanup_exit_entry('{mode}')._native1()"),
                vec![],
                PrintWriter::Stdout,
            )
            .unwrap();
        for (phase, trace) in [("inner", "['inner']"), ("outer", "['inner', 'outer']")] {
            let call = progress.into_function_call().unwrap();
            let observation: serde_json::Value =
                serde_json::from_str(call.args.arg(0).unwrap().as_str().unwrap()).unwrap();
            assert_eq!(observation["phase"], phase);
            progress = call.resume_pending(PrintWriter::Stdout).unwrap();
            let bytes = dump("policy7-native6.py", None, &[], SessionRef::Suspended(&progress)).unwrap();
            let Session::Suspended(copy) = Dump::load(&bytes).unwrap().state else {
                panic!("pending")
            };
            let (disposed, continuation) = if restoring {
                let Session::Suspended(continuation) = Dump::load(&bytes).unwrap().state else {
                    panic!("pending")
                };
                (vec![progress, *copy], *continuation)
            } else {
                (vec![*copy], progress)
            };
            for pending in disposed {
                let mut repl = pending.into_repl();
                repl.feed_run(
                    &format!("assert TRACE == {trace}\nTRACE = []\npulse = []\npulse = None"),
                    vec![],
                    PrintWriter::Stdout,
                )
                .unwrap();
                assert_eq!(
                    repl.heap_entry_count(),
                    baseline,
                    "{mode}/{phase}: disposal before teardown"
                );
            }
            let pending = continuation.into_resolve_futures().unwrap();
            let id = pending.pending_call_ids()[0];
            let reply = if mode == "cleanup_error" && phase == "inner" {
                ExtFunctionResult::Error(MontyException::new(ExcType::TypeError, Some("CLEANUP".into())))
            } else {
                ExtFunctionResult::Return(MontyObject::string("null"))
            };
            progress = pending
                .resume_native(
                    vec![(id, reply)],
                    Some(if phase == "inner" { "" } else { "REPEAT" }.into()),
                    PrintWriter::Stdout,
                )
                .unwrap();
        }
        let (mut repl, value) = progress.into_complete().unwrap();
        drop(value);
        let trace = "['inner', 'outer']";
        repl.feed_run(
            &format!("assert TRACE == {trace}\nTRACE = []\npulse = []\npulse = None"),
            vec![],
            PrintWriter::Stdout,
        )
        .unwrap();
        eprintln!(
            "policy7-native6/{mode}/restoring={restoring}: baseline={baseline}, completed={}",
            repl.heap_entry_count()
        );
        assert_eq!(repl.heap_entry_count(), baseline, "{mode}: terminal before teardown");
    }
}

/// New policy7 lifecycle coverage, using literal expectations frozen before execution.
#[test]
fn native7_repair10_uncaught_and_later_census() {
    for mode in ["uncaught", "later"] {
        repair10_census(mode);
    }
}

#[cfg(feature = "cancellation-test-hooks")]
#[test]
fn native7_repair10_fatal_and_depth_census() {
    for mode in [
        "depth",
        "terminal",
        "delivery-internal",
        "delivery-uncatchable",
        "projection-internal",
        "projection-uncatchable",
    ] {
        repair10_census(mode);
    }
}

fn repair10_census(mode: &str) {
    for restoring in [false, true] {
        let mut repl = new_repl();
        repl.feed_run(
            include_str!("../../../tools/coroutines/cancelled_error/migrated/catchable_cancellation10/ownership.py"),
            vec![("exchange".into(), MontyObject::function("exchange", None))],
            PrintWriter::Stdout,
        )
        .unwrap();
        let baseline = repl.heap_entry_count();
        let mut progress = Some(
            repl.feed_start(&format!("repair10('{mode}')._native1()"), vec![], PrintWriter::Stdout)
                .unwrap(),
        );
        let phases: &[&str] = if mode == "uncaught" || mode.starts_with("delivery-") {
            &["cleanup"]
        } else {
            &["cleanup", "recovery"]
        };
        let mut failure = None;
        for phase in phases {
            let call = progress.take().unwrap().into_function_call().unwrap();
            assert_eq!(call.args.arg(0).unwrap().as_str(), Some(*phase));
            let suspended = call.resume_pending(PrintWriter::Stdout).unwrap();
            let bytes = dump("repair10.py", None, &[], SessionRef::Suspended(&suspended)).unwrap();
            let Session::Suspended(copy) = Dump::load(&bytes).unwrap().state else {
                panic!("suspended")
            };
            let (disposed, continuation) = if restoring {
                (suspended, *copy)
            } else {
                (*copy, suspended)
            };
            let mut disposed = disposed.into_repl();
            let trace = if *phase == "cleanup" { "[]" } else { "['caught']" };
            disposed
                .feed_run(
                    &format!("assert TRACE == {trace}\nTRACE = []\npulse = []\npulse = None"),
                    vec![],
                    PrintWriter::Stdout,
                )
                .unwrap();
            assert_eq!(
                disposed.heap_entry_count(),
                baseline,
                "{mode}/{phase}: disposal before teardown"
            );
            let pending = continuation.into_resolve_futures().unwrap();
            let id = pending.pending_call_ids()[0];
            #[cfg(feature = "cancellation-test-hooks")]
            if mode.starts_with("delivery-") || *phase == "recovery" {
                monty::cancellation_test_hooks::configure(if mode == "later" { "" } else { mode });
            }
            match pending.resume_native(
                vec![(id, ExtFunctionResult::Return(MontyObject::none()))],
                Some(if *phase == "cleanup" { "FIRST" } else { "REPEAT" }.into()),
                PrintWriter::Stdout,
            ) {
                Ok(next) => progress = Some(next),
                Err(error) => {
                    failure = Some(error);
                    break;
                }
            }
        }
        let mut repl = if let Some(failure) = failure {
            assert!(
                mode.contains("internal") || mode.contains("uncatchable") || mode == "terminal",
                "{mode}: {}",
                failure.error
            );
            let message = failure.error.to_string();
            let marker = if mode.contains("internal") {
                "REPAIR10_INTERNAL"
            } else if mode.contains("uncatchable") {
                "REPAIR10_UNCATCHABLE"
            } else {
                "REPAIR10_EXPORT_TERMINAL"
            };
            assert!(message.contains(marker), "{message}");
            assert!(!message.contains("asyncio.CancelledError: FIRST"));
            failure.repl
        } else {
            assert!(
                matches!(mode, "uncaught" | "later" | "depth"),
                "fatal must not Complete: {mode}"
            );
            let (repl, value) = progress.unwrap().into_complete().unwrap();
            let fields = value.as_ref().pairs().unwrap();
            let field = |name| fields.iter().find(|(key, _)| key.as_str() == Some(name)).unwrap().1;
            assert_eq!(field("status").as_str(), Some("cancelled"));
            assert_eq!(field("reason").as_str(), Some("FIRST"));
            assert_eq!(
                field("value_export_error").as_str(),
                if mode == "depth" {
                    Some("export-depth-limit")
                } else {
                    None
                }
            );
            if mode != "depth" {
                assert_eq!(
                    field("error").as_str().unwrap().lines().last(),
                    Some(if mode == "uncaught" {
                        "asyncio.CancelledError: FIRST"
                    } else {
                        "TypeError: LATER"
                    })
                );
            }
            drop(value);
            repl
        };
        #[cfg(feature = "cancellation-test-hooks")]
        if matches!(mode, "depth" | "terminal") {
            let stats = monty::cancellation_test_hooks::export_stats();
            assert!(stats.visits > 1 && stats.peak_pins > 0 && stats.dropped, "{stats:?}");
            assert_eq!(stats.peak_pins, stats.retired_pins);
        }
        #[cfg(feature = "cancellation-test-hooks")]
        monty::cancellation_test_hooks::configure("");
        let trace = if mode.starts_with("delivery-") {
            "[]"
        } else {
            "['caught']"
        };
        repl.feed_run(
            &format!("assert TRACE == {trace}\nTRACE = []\npulse = []\npulse = None"),
            vec![],
            PrintWriter::Stdout,
        )
        .unwrap();
        eprintln!(
            "repair10/{mode}/{restoring}: baseline={baseline}, completed={}",
            repl.heap_entry_count()
        );
        assert_eq!(repl.heap_entry_count(), baseline, "{mode}: terminal before teardown");
    }
}

/// Rebuilt genuine Dump17 frozen cache and coroutine/generator code, never patched metadata.
#[test]
fn native7_compatibility11_historical_activation() {
    assert!(matches!(
        Dump::load(include_bytes!("fixtures/compatibility11_frozen_v17.bin")),
        Err(monty::DumpError::VersionTooOld { .. })
    ));
}

/// The concurrent-I/O slice uses one host-mode parent and two Rust-owned child
/// generator chains. Verify live ownership before heap teardown, including the
/// partially joined cut and repeated lifecycles on the same REPL.
#[test]
fn concurrent_io_live_ownership() {
    let mut repl = new_repl();
    repl.feed_run(
        include_str!("../../../tools/coroutines/concurrent_io/wrappers.py"),
        vec![],
        PrintWriter::Stdout,
    )
    .unwrap();
    repl.feed_run(
        "from __monty_host__ import host_coroutines\nimport json\nCLEAN = []\ndef request(label):\n    return (yield exchange(label))\nasync def operation(label):\n    cycle = []\n    cycle.append(cycle)\n    try:\n        return [label, await request(label)]\n    finally:\n        CLEAN.append(label)\nasync def parent():\n    return await asyncio.gather(child(operation('A')), child(operation('B')))",
        vec![("exchange".into(), MontyObject::function("exchange", None))],
        PrintWriter::Stdout,
    ).unwrap();
    let baseline = repl.heap_entry_count();
    for iteration in 0..12 {
        let mut progress = repl
            .feed_start("parent()._native1()", vec![], PrintWriter::Stdout)
            .unwrap();
        let mut ids = Vec::new();
        while let ReplProgress::FunctionCall(call) = progress {
            ids.push(call.call_id);
            progress = call.resume_pending(PrintWriter::Stdout).unwrap();
        }
        assert_eq!(ids.len(), 2);
        for cut in 0..2 {
            let bytes = dump("concurrent_io.py", None, &[], SessionRef::Suspended(&progress)).unwrap();
            // Independent restoration and abandonment must release cycles and
            // gather/result records while leaving the original branch usable.
            let Session::Suspended(copy) = Dump::load(&bytes).unwrap().state else {
                panic!("suspended")
            };
            let (abandon, active) = if iteration % 2 == 0 {
                (*copy, progress)
            } else {
                (progress, *copy)
            };
            progress = active;
            let mut abandoned = abandon.into_repl();
            abandoned
                .feed_run(
                    if cut == 0 {
                        "assert CLEAN == []"
                    } else {
                        "assert CLEAN == ['B']"
                    },
                    vec![],
                    PrintWriter::Stdout,
                )
                .unwrap();
            abandoned
                .feed_run("pulse = []\npulse = None", vec![], PrintWriter::Stdout)
                .unwrap();
            assert_eq!(abandoned.heap_entry_count(), baseline);
            let pending = progress.into_resolve_futures().unwrap();
            assert_eq!(pending.pending_call_ids().len(), 2 - cut);
            progress = pending
                .resume(
                    vec![(
                        ids[1 - cut],
                        ExtFunctionResult::Return(MontyObject::string(if cut == 0 { "beta" } else { "alpha" })),
                    )],
                    PrintWriter::Stdout,
                )
                .unwrap();
        }
        let (mut completed, value) = progress.into_complete().unwrap();
        assert_eq!(
            value,
            MontyObject::list([
                MontyObject::list([MontyObject::string("A"), MontyObject::string("alpha")]),
                MontyObject::list([MontyObject::string("B"), MontyObject::string("beta")]),
            ])
        );
        completed
            .feed_run(
                "assert CLEAN == ['B', 'A']\nCLEAN.clear()\npulse = []\npulse = None",
                vec![],
                PrintWriter::Stdout,
            )
            .unwrap();
        assert_eq!(completed.heap_entry_count(), baseline);
        repl = completed;
    }
}

/// Cancellation/drain roots must disappear while the REPL heap is still live.
#[test]
fn concurrent_io_child_scope_ownership() {
    for fixture in [
        include_str!("../../../tools/coroutines/cancelled_error/migrated/concurrent_io/children_drain.py"),
        include_str!("../../../tools/coroutines/concurrent_io/children_errors.py"),
        include_str!("../../../tools/coroutines/cancelled_error/migrated/concurrent_io/children_cancel.py"),
        include_str!("../../../tools/coroutines/cancelled_error/migrated/concurrent_io/host_cancel.py"),
        include_str!("../../../tools/coroutines/cancelled_error/migrated/concurrent_io/deferred_cleanup.py"),
        include_str!("../../../tools/coroutines/cancelled_error/migrated/concurrent_io/gather_host.py"),
        include_str!("../../../tools/coroutines/concurrent_io/gather_first_error.py"),
    ] {
        let mut repl = new_repl();
        repl.feed_run(
            include_str!("../../../tools/coroutines/concurrent_io/wrappers.py"),
            vec![],
            PrintWriter::Stdout,
        )
        .unwrap();
        repl.feed_run(
            include_str!("../../../tools/coroutines/concurrent_io/scope.py"),
            vec![],
            PrintWriter::Stdout,
        )
        .unwrap();
        let definitions = fixture.replace("parent()._native1()", "").replace(
            "async def operation(label):",
            "async def operation(label):\n    cycle = []\n    cycle.append(cycle)",
        );
        repl.feed_run(
            &definitions,
            vec![("exchange".into(), MontyObject::function("exchange", None))],
            PrintWriter::Stdout,
        )
        .unwrap();
        let baseline = repl.heap_entry_count();
        for _ in 0..12 {
            let mut progress = repl
                .feed_start("parent()._native1()", vec![], PrintWriter::Stdout)
                .unwrap();
            let mut calls = Vec::new();
            let mut first_turn = true;
            loop {
                while let ReplProgress::FunctionCall(call) = progress {
                    calls.push((call.call_id, call.args.arg(0).unwrap().as_str().unwrap().to_owned()));
                    progress = call.resume_pending(PrintWriter::Stdout).unwrap();
                }
                if matches!(progress, ReplProgress::Complete { .. }) {
                    break;
                }
                let bytes = dump("children.py", None, &[], SessionRef::Suspended(&progress)).unwrap();
                let Session::Suspended(copy) = Dump::load(&bytes).unwrap().state else {
                    panic!("suspended")
                };
                let mut abandoned = copy.into_repl();
                abandoned
                    .feed_run("TRACE = []\npulse = []\npulse = None", vec![], PrintWriter::Stdout)
                    .unwrap();
                // A cancellation fixture without TRACE defines it here; account for that empty list.
                assert_eq!(
                    abandoned.heap_entry_count(),
                    baseline + usize::from(!fixture.contains("TRACE = []"))
                );
                let pending = progress.into_resolve_futures().unwrap();
                let ids = pending.pending_call_ids();
                let selected = calls
                    .iter()
                    .find(|(id, label)| {
                        ids.contains(id) && (label == "exit" || label == "cancel" || label == "continue")
                    })
                    .or_else(|| {
                        calls
                            .iter()
                            .find(|(id, label)| ids.contains(id) && label.starts_with("cleanup-"))
                    })
                    .or_else(|| calls.iter().find(|(id, _)| ids.contains(id)))
                    .unwrap()
                    .0;
                progress = pending
                    .resume_native(
                        if first_turn
                            && (fixture.contains("scope.defer")
                                || fixture.contains("['caught', label")
                                || fixture.contains("async def branch():"))
                        {
                            Vec::new()
                        } else {
                            vec![(selected, ExtFunctionResult::Return(MontyObject::string("beta")))]
                        },
                        (fixture.contains("scope.defer")
                            || fixture.contains("['caught', label")
                            || fixture.contains("async def branch():"))
                        .then(|| "FIRST".to_owned()),
                        PrintWriter::Stdout,
                    )
                    .unwrap();
                first_turn = false;
            }
            let (mut completed, _) = progress.into_complete().unwrap();
            completed
                .feed_run(
                    if fixture.contains("TRACE = []") {
                        "TRACE.clear()\npulse = []\npulse = None"
                    } else {
                        "pulse = []\npulse = None"
                    },
                    vec![],
                    PrintWriter::Stdout,
                )
                .unwrap();
            assert_eq!(completed.heap_entry_count(), baseline);
            repl = completed;
        }
    }
}

/// Public authoring acceptance: real imports, variadic starred gathering and
/// snapshot-owned host requests, with no authored bridge calls.
#[test]
fn standard_python_host_reads() {
    let repl = new_repl();
    let source = concat!(
        "from __monty_host__ import host_coroutines\n",
        "import asyncio\nfrom sandbox import read_file\n",
        "async def main():\n",
        "    paths = ['a.txt', 'b.txt', 'c.txt', 'd.txt', 'e.txt']\n",
        "    return await asyncio.gather(*(read_file(path) for path in paths))\n",
        "main()._native1()",
    );
    let mut progress = repl.feed_start(source, vec![], PrintWriter::Stdout).unwrap();
    let mut ids = Vec::new();
    while let ReplProgress::FunctionCall(call) = progress {
        assert_eq!(call.function_name, "sandbox.exchange");
        ids.push(call.call_id);
        progress = call.resume_pending(PrintWriter::Stdout).unwrap();
    }
    assert_eq!(ids.len(), 5);
    for index in (0..5).rev() {
        let bytes = dump("reads.py", None, &[], SessionRef::Suspended(&progress)).unwrap();
        drop(progress.into_repl());
        let Session::Suspended(restored) = Dump::load(&bytes).unwrap().state else {
            panic!("expected suspended reads")
        };
        let pending = restored.into_resolve_futures().unwrap();
        let mut actual = pending.pending_call_ids().to_vec();
        actual.sort_unstable();
        assert_eq!(actual, ids[..=index]);
        progress = pending
            .resume(
                vec![(
                    ids[index],
                    ExtFunctionResult::Return(MontyObject::string(index.to_string())),
                )],
                PrintWriter::Stdout,
            )
            .unwrap();
    }
    let (_, value) = progress.into_complete().unwrap();
    assert_eq!(
        value,
        MontyObject::list((0..5).map(|index| MontyObject::string(index.to_string())))
    );
}

#[test]
fn playground_capabilities_restore_and_release() {
    let repl = new_repl();
    let source = concat!(
        "from __monty_host__ import host_coroutines\n",
        "import asyncio\nfrom sandbox import ask, random, echo, sample, decision, document_create, document_read, document_edit, present_choices, display_document\n",
        "async def main():\n",
        "    return await asyncio.gather(*(fn({'message': 'owned'}) for fn in [ask, random, echo, sample, decision, document_create, document_read, document_edit, present_choices, display_document]))\n",
        "main()._native1()",
    );
    let mut progress = repl.feed_start(source, vec![], PrintWriter::Stdout).unwrap();
    let mut ids = Vec::new();
    while let ReplProgress::FunctionCall(call) = progress {
        assert_eq!(call.function_name, "sandbox.exchange");
        ids.push(call.call_id);
        progress = call.resume_pending(PrintWriter::Stdout).unwrap();
    }
    assert_eq!(ids.len(), 10);
    let bytes = dump("playground.py", None, &[], SessionRef::Suspended(&progress)).unwrap();
    drop(progress.into_repl());
    let Session::Suspended(restored) = Dump::load(&bytes).unwrap().state else {
        panic!("expected pending capabilities")
    };
    let pending = restored.into_resolve_futures().unwrap();
    let result = pending
        .resume(
            ids.into_iter()
                .map(|id| (id, ExtFunctionResult::Return(MontyObject::string("ok"))))
                .collect(),
            PrintWriter::Stdout,
        )
        .unwrap();
    let (_, value) = result.into_complete().unwrap();
    assert_eq!(value, MontyObject::list((0..10).map(|_| MontyObject::string("ok"))));
}

/// Ordinary coroutines release context and exception owners before heap teardown.
#[test]
fn native_async_context_live_ownership() {
    let output = checked_run(
        r"
import asyncio
token = []
token.append(token)
class C:
    def __init__(self, value):
        self.value = value
    async def __aenter__(self):
        return self.value
    async def __aexit__(self, kind, value, tb):
        if value is not None:
            assert value.args[0] is token
        return True
async def work(mode):
    async with C(token) as value:
        if mode == 'error':
            raise ValueError(value)
        return value
for mode in ['return', 'error']:
    result = asyncio.run(work(mode))
    assert result is token if mode == 'return' else result is None
result = mode = work = C = None
None
",
    );
    assert_eq!(output.counts["token"], 2, "global and self edge only");
}

/// Abandoning original and restored native context frames releases cyclic locals.
#[test]
fn native_async_context_snapshot_disposal_ownership() {
    for stage in ["entry", "body", "exit"] {
        let mut repl = new_repl();
        repl.feed_run(
            r"
import asyncio
class C:
    async def __aenter__(self):
        self.value = []
        self.value.append(self.value)
        if stage == 'entry':
            host()
        return self
    async def __aexit__(self, kind, value, tb):
        if stage == 'exit':
            host()
async def work():
    async with C() as value:
        if stage == 'body':
            host()
stage = None
",
            vec![("host".to_owned(), MontyObject::function("host", None))],
            PrintWriter::Stdout,
        )
        .unwrap();
        let baseline = repl.heap_entry_count();
        let source = format!("stage = '{stage}'\nasyncio.run(work())");
        let progress = repl.feed_start(&source, vec![], PrintWriter::Stdout).unwrap();
        assert!(matches!(progress, ReplProgress::FunctionCall(_)));
        let bytes = dump("context.py", None, &[], SessionRef::Suspended(&progress)).unwrap();
        let Session::Suspended(restored) = Dump::load(&bytes).unwrap().state else {
            panic!("suspended context")
        };
        for (kind, progress) in [("original", progress), ("restored", *restored)] {
            let mut repl = progress.into_function_call().unwrap().into_repl();
            repl.feed_run("stage = None\npulse = []\npulse = None", vec![], PrintWriter::Stdout)
                .unwrap();
            assert_eq!(repl.heap_entry_count(), baseline, "{stage}/{kind}: before teardown");
        }
    }
}

/// Ordinary coroutine cancellation retains protected replies and releases locals
/// on completion and independent snapshot disposal, before heap teardown.
#[test]
fn native_async_cancel_snapshot_ownership() {
    for (first_cut, context) in [("body", false), ("exit", false), ("body", true), ("exit", true)] {
        let mut repl = new_repl();
        repl.feed_run(
            r"
import asyncio
class C:
    def __init__(self, token):
        self.token = token
    async def __aenter__(self):
        return self
    async def __aexit__(self, kind, value, tb):
        assert await exchange('exit') == 'reply'
async def managed():
    token = []
    token.append(token)
    async with C(token):
        await exchange('body')
    return 'done'
async def work():
    token = []
    token.append(token)
    try:
        await exchange('body')
    finally:
        assert await exchange('exit') == 'reply'
    return 'done'
",
            vec![("exchange".into(), MontyObject::function("exchange", None))],
            PrintWriter::Stdout,
        )
        .unwrap();
        let baseline = repl.heap_entry_count();
        let entry = if context {
            "asyncio.run(managed())"
        } else {
            "asyncio.run(work())"
        };
        let mut progress = repl.feed_start(entry, vec![], PrintWriter::Stdout).unwrap();
        for phase in ["body", "exit"] {
            let call = progress.into_function_call().unwrap();
            assert_eq!(call.args.arg(0).unwrap().as_str(), Some(phase));
            progress = call.resume_pending(PrintWriter::Stdout).unwrap();
            let bytes = dump("cancel.py", None, &[], SessionRef::Suspended(&progress)).unwrap();
            for kind in ["copy1", "copy2"] {
                let Session::Suspended(copy) = Dump::load(&bytes).unwrap().state else {
                    panic!("pending")
                };
                let mut abandoned = copy.into_resolve_futures().unwrap().into_repl();
                abandoned
                    .feed_run("pulse = []\npulse = None", vec![], PrintWriter::Stdout)
                    .unwrap();
                assert_eq!(abandoned.heap_entry_count(), baseline, "{first_cut}/{phase}/{kind}");
            }
            // Dispose the original independently, then advance a fresh restoration.
            let mut abandoned = progress.into_resolve_futures().unwrap().into_repl();
            abandoned
                .feed_run("pulse = []\npulse = None", vec![], PrintWriter::Stdout)
                .unwrap();
            assert_eq!(abandoned.heap_entry_count(), baseline, "original/{phase}");
            let Session::Suspended(copy) = Dump::load(&bytes).unwrap().state else {
                panic!("pending")
            };
            let pending = copy.into_resolve_futures().unwrap();
            let id = pending.pending_call_ids()[0];
            let cancelling = phase == first_cut || first_cut == "body";
            let results = if phase == "body" && cancelling {
                vec![]
            } else {
                vec![(id, ExtFunctionResult::Return(MontyObject::string("reply")))]
            };
            progress = pending
                .resume_native(
                    results,
                    cancelling.then(|| if phase == first_cut { "first" } else { "repeat" }.into()),
                    PrintWriter::Stdout,
                )
                .unwrap();
        }
        let (mut repl, value) = progress.into_complete().unwrap();
        repl.feed_run(
            "assert outcome['status'] == 'cancelled'\nassert outcome['reason'] == 'first'\nassert outcome['value_export_error'] is None\noutcome = None\npulse = []\npulse = None",
            vec![("outcome".into(), value)],
            PrintWriter::Stdout,
        ).unwrap();
        assert_eq!(repl.heap_entry_count(), baseline, "completion/{first_cut}");
    }
}

/// Concurrent host cancellation drains admitted work; the genuine pre-request
/// snapshot remains independently usable for ordinary settlement.
#[test]
fn native_async_cancel_drains_concurrent_and_preserves_original() {
    for operands in ["child(), child()", "exchange('a'), exchange('b')", "public-task-join"] {
        let body = if operands == "public-task-join" {
            "[await asyncio.create_task(child()), 7]".to_owned()
        } else {
            format!("await asyncio.gather({operands})")
        };
        let repl = new_repl();
        let source = format!(
            "import asyncio\nasync def child():\n    return await exchange('child')\nasync def main():\n    return {body}\nasyncio.run(main())"
        );
        let mut progress = repl
            .feed_start(
                &source,
                vec![("exchange".into(), MontyObject::function("exchange", None))],
                PrintWriter::Stdout,
            )
            .unwrap();
        while let ReplProgress::FunctionCall(call) = progress {
            progress = call.resume_pending(PrintWriter::Stdout).unwrap();
        }
        let bytes = dump("concurrent.py", None, &[], SessionRef::Suspended(&progress)).unwrap();
        let (_, value) = progress
            .into_resolve_futures()
            .unwrap()
            .resume_native(vec![], Some("host".into()), PrintWriter::Stdout)
            .unwrap()
            .into_complete()
            .unwrap();
        assert!(format!("{value:?}").contains("cancelled"));
        let Session::Suspended(copy) = Dump::load(&bytes).unwrap().state else {
            panic!("pending")
        };
        let pending = copy.into_resolve_futures().unwrap();
        let replies = pending
            .pending_call_ids()
            .iter()
            .map(|id| (*id, ExtFunctionResult::Return(MontyObject::int(7))))
            .collect();
        let (_, value) = pending
            .resume(replies, PrintWriter::Stdout)
            .unwrap()
            .into_complete()
            .unwrap();
        assert_eq!(value, MontyObject::list([MontyObject::int(7), MontyObject::int(7)]));
    }
}

/// Guest recovery cannot reopen child admission or clear the host's first cause.
#[test]
fn native_async_cancel_caught_is_terminal() {
    let repl = new_repl();
    let progress = repl.feed_start(
        "import asyncio\nasync def main():\n    try:\n        await exchange()\n    except asyncio.CancelledError:\n        try:\n            await asyncio.gather()\n        except ValueError as e:\n            assert str(e) == 'child admission is closed'\n        return 'caught'\nasyncio.run(main())",
        vec![("exchange".into(), MontyObject::function("exchange", None))],
        PrintWriter::Stdout,
    ).unwrap().into_function_call().unwrap().resume_pending(PrintWriter::Stdout).unwrap();
    // Use an empty gather to test admission without evaluating another host call.
    let pending = progress.into_resolve_futures().unwrap();
    let progress = pending
        .resume_native(vec![], Some("first".into()), PrintWriter::Stdout)
        .unwrap();
    let (_, value) = progress.into_complete().unwrap();
    assert_eq!(
        value,
        MontyObject::dict([
            (MontyObject::string("status"), MontyObject::string("cancelled")),
            (MontyObject::string("reason"), MontyObject::string("first")),
            (MontyObject::string("value"), MontyObject::string("caught")),
            (MontyObject::string("error"), MontyObject::none()),
            (MontyObject::string("interrupted_error"), MontyObject::none()),
            (MontyObject::string("value_export_error"), MontyObject::none()),
        ])
    );
}

/// Protected ordinary gathers retain the first error until every child cleanup
/// completes. Every cut is independently disposable and freshly resumable.
#[test]
fn native_async_cleanup_gather_error_barrier_ownership() {
    for (cancelled, mode) in [
        (false, "flat"),
        (false, "nested"),
        (false, "commit"),
        (true, "flat"),
        (true, "nested"),
        (true, "commit"),
        (false, "public"),
    ] {
        let mut repl = new_repl();
        repl.feed_run(
            r"
import asyncio
async def left():
    token = []
    token.append(token)
    await exchange('A')
    raise ValueError('left-error')
async def right():
    token = []
    token.append(token)
    try:
        assert await exchange('B') == 'reply'
    finally:
        assert await exchange('child-release') == 'reply'
async def inner():
    try:
        pass
    finally:
        await asyncio.gather(left(), right())
async def done():
    return None
async def main():
    try:
        await exchange('body')
    finally:
        try:
            if nested:
                await asyncio.gather(inner())
            elif commit:
                used = done()
                await used
                await asyncio.gather(right(), used)
            elif public:
                first = asyncio.create_task(left())
                second = asyncio.create_task(right())
                await asyncio.gather(first, second)
            else:
                await asyncio.gather(left(), right())
        finally:
            assert await exchange('parent-release') == 'reply'
nested = None
commit = None
public = None
",
            vec![("exchange".into(), MontyObject::function("exchange", None))],
            PrintWriter::Stdout,
        )
        .unwrap();
        repl.feed_run(
            &format!(
                "nested = {}\ncommit = {}\npublic = {}",
                if mode == "nested" { "True" } else { "False" },
                if mode == "commit" { "True" } else { "False" },
                if mode == "public" { "True" } else { "False" }
            ),
            vec![],
            PrintWriter::Stdout,
        )
        .unwrap();
        let baseline = repl.heap_entry_count();
        let mut progress = repl
            .feed_start("asyncio.run(main())", vec![], PrintWriter::Stdout)
            .unwrap();
        let mut calls = Vec::new();
        let mut cuts = vec![
            (vec!["body"], "body"),
            (vec!["A", "B"], "A"),
            (vec![], "B"),
            (vec!["child-release"], "child-release"),
            (vec!["parent-release"], "parent-release"),
        ];
        if mode == "commit" {
            cuts[1] = (vec!["B"], "B");
            cuts.remove(2);
        }
        for (expected_calls, reply) in cuts {
            let mut labels = Vec::new();
            while let ReplProgress::FunctionCall(call) = progress {
                let label = call.args.arg(0).unwrap().as_str().unwrap().to_owned();
                labels.push(label.clone());
                calls.push((label, call.call_id));
                progress = call.resume_pending(PrintWriter::Stdout).unwrap();
            }
            assert_eq!(
                labels, expected_calls,
                "cancelled={cancelled}, mode={mode}, reply={reply}"
            );
            let bytes = dump("barrier.py", None, &[], SessionRef::Suspended(&progress)).unwrap();
            // Original and two independent restored abandonments release all
            // continuation, cached gather error and Ready(error) owners.
            for copy in 0..3 {
                let abandoned = progress;
                let mut disposed = abandoned.into_resolve_futures().unwrap().into_repl();
                disposed
                    .feed_run("pulse = []\npulse = None", vec![], PrintWriter::Stdout)
                    .unwrap();
                assert_eq!(disposed.heap_entry_count(), baseline, "abandon {reply}/{copy}");
                let Session::Suspended(restored) = Dump::load(&bytes).unwrap().state else {
                    panic!("pending")
                };
                progress = *restored;
            }
            let pending = progress.into_resolve_futures().unwrap();
            let id = calls.iter().find(|(label, _)| label == reply).unwrap().1;
            let results = if cancelled && reply == "body" {
                vec![]
            } else {
                vec![(id, ExtFunctionResult::Return(MontyObject::string("reply")))]
            };
            let advanced = pending.resume_native(
                results,
                cancelled.then(|| if reply == "body" { "FIRST" } else { "REPEAT" }.into()),
                PrintWriter::Stdout,
            );
            if reply == "parent-release" && !cancelled {
                let error = advanced.unwrap_err();
                assert_eq!(
                    error.error.exc_type(),
                    if mode == "commit" {
                        ExcType::RuntimeError
                    } else {
                        ExcType::ValueError
                    }
                );
                assert_eq!(
                    error.error.message(),
                    Some(if mode == "commit" {
                        "cannot reuse already awaited coroutine"
                    } else {
                        "left-error"
                    })
                );
                let mut finished = error.repl;
                finished
                    .feed_run("pulse = []\npulse = None", vec![], PrintWriter::Stdout)
                    .unwrap();
                assert_eq!(finished.heap_entry_count(), baseline, "normal error completion");
                // Keep a harmless completed progress for the loop's move analysis.
                progress = finished.feed_start("None", vec![], PrintWriter::Stdout).unwrap();
            } else {
                progress = advanced.unwrap();
            }
        }
        let (mut repl, value) = progress.into_complete().unwrap();
        if cancelled {
            let error = if mode == "commit" {
                "RuntimeError: cannot reuse already awaited coroutine"
            } else {
                "ValueError: left-error"
            };
            repl.feed_run(
                    &format!("assert outcome['status'] == 'cancelled'\nassert outcome['reason'] == 'FIRST'\nassert outcome['error'].endswith('{error}')\noutcome = None"),
                    vec![("outcome".into(), value)],
                    PrintWriter::Stdout,
                ).unwrap();
        }
        repl.feed_run("pulse = []\npulse = None", vec![], PrintWriter::Stdout)
            .unwrap();
        assert_eq!(repl.heap_entry_count(), baseline, "terminal completion");
    }
}

/// Public cached outcomes, aliases and bound inspection methods release their
/// roots before teardown, including cyclic values and exception identities.
#[test]
fn native_async_public_tasks_live_ownership() {
    let output = checked_run(
        r"
import asyncio
token = []
token.append(token)
async def work(fail):
    if fail:
        raise ValueError(token)
    return token
async def main():
    task = asyncio.create_task(work(False))
    alias = task
    values = await asyncio.gather(task, task)
    assert values[0] is token and values[1] is token
    assert await alias is token
    method = task.result
    task = alias = None
    assert method() is token
    bad = asyncio.create_task(work(True))
    for i in range(3):
        try:
            await bad
        except ValueError as error:
            assert error is bad.exception()
            assert error.args[0] is token
    asyncio.create_task(work(True))
    await asyncio.create_task(work(False))
asyncio.run(main())
main = work = None
None
",
    );
    assert_eq!(output.counts["token"], 2, "global plus self edge only");
}

/// Every original/restored public-task suspension can be independently disposed;
/// continued execution also releases handle, waiter and cached exception roots.
#[test]
fn native_async_public_tasks_snapshot_ownership() {
    let mut repl = new_repl();
    repl.feed_run(
        r"
import asyncio
async def child(label, token):
    await exchange(label)
    if label == 'B':
        raise ValueError(token)
    return token
async def join(task):
    return await task
async def main():
    token = []
    token.append(token)
    first = asyncio.create_task(child('A', token))
    asyncio.create_task(child('B', token))
    values = await asyncio.gather(join(first), join(first), first)
    assert all(value is token for value in values)
    assert await first is token and first.result() is token
    await exchange('last')
    return None
",
        vec![("exchange".into(), MontyObject::function("exchange", None))],
        PrintWriter::Stdout,
    )
    .unwrap();
    let baseline = repl.heap_entry_count();
    let mut progress = repl
        .feed_start("asyncio.run(main())", vec![], PrintWriter::Stdout)
        .unwrap();
    let mut cuts = 0;
    while !matches!(progress, ReplProgress::Complete { .. }) {
        cuts += 1;
        assert!(cuts < 12, "bounded task trace");
        let bytes = dump("tasks.py", None, &[], SessionRef::Suspended(&progress)).unwrap();
        for _ in 0..2 {
            let Session::Suspended(copy) = Dump::load(&bytes).unwrap().state else {
                panic!("suspended")
            };
            let mut abandoned = copy.into_repl();
            abandoned
                .feed_run("pulse = []\npulse = None", vec![], PrintWriter::Stdout)
                .unwrap();
            assert_eq!(abandoned.heap_entry_count(), baseline, "restored cut {cuts}");
        }
        let mut abandoned = progress.into_repl();
        abandoned
            .feed_run("pulse = []\npulse = None", vec![], PrintWriter::Stdout)
            .unwrap();
        assert_eq!(abandoned.heap_entry_count(), baseline, "original cut {cuts}");
        let Session::Suspended(copy) = Dump::load(&bytes).unwrap().state else {
            panic!("suspended")
        };
        progress = if matches!(*copy, ReplProgress::FunctionCall(_)) {
            copy.into_function_call()
                .unwrap()
                .resume_pending(PrintWriter::Stdout)
                .unwrap()
        } else {
            let pending = copy.into_resolve_futures().unwrap();
            let ids = pending.pending_call_ids().to_vec();
            pending
                .resume(
                    ids.into_iter()
                        .map(|id| (id, ExtFunctionResult::Return(MontyObject::none())))
                        .collect(),
                    PrintWriter::Stdout,
                )
                .unwrap()
        };
    }
    let (mut repl, _) = progress.into_complete().unwrap();
    repl.feed_run("pulse = []\npulse = None", vec![], PrintWriter::Stdout)
        .unwrap();
    assert_eq!(repl.heap_entry_count(), baseline, "completed before teardown");
}

/// Cancelling a public join preserves child-before-waiter cleanup and releases
/// live cancellation signals, stale external registrations and waiter routes.
#[test]
fn native_async_public_task_cancel_snapshot_ownership() {
    for (program, replies) in [
        (
            include_str!("../../../tools/coroutines/cancelled_error/migrated/native_async_tasks/cases/cancel.py"),
            vec!["control", "again", "release", "waiter-release"],
        ),
        (
            include_str!(
                "../../../tools/coroutines/cancelled_error/migrated/native_async_tasks/cases/cancel-gather.py"
            ),
            vec!["control", "A-release", "B-release", "owner-release"],
        ),
        (
            include_str!(
                "../../../tools/coroutines/cancelled_error/migrated/native_async_tasks/cases/cancel-gather-wrapped.py"
            ),
            vec!["control", "A-release", "B-release", "owner-release"],
        ),
    ] {
        let source = program.replace("asyncio.run(main())", "");
        let mut repl = new_repl();
        repl.feed_run(
            &source,
            vec![("exchange".into(), MontyObject::function("exchange", None))],
            PrintWriter::Stdout,
        )
        .unwrap();
        let baseline = repl.heap_entry_count();
        let mut progress = repl
            .feed_start("asyncio.run(main())", vec![], PrintWriter::Stdout)
            .unwrap();
        let mut identities = HashMap::new();
        let mut phase = 0;
        let mut cuts = 0;
        while !matches!(progress, ReplProgress::Complete { .. }) {
            cuts += 1;
            assert!(cuts < 20);
            let bytes = dump("cancel-task.py", None, &[], SessionRef::Suspended(&progress)).unwrap();
            for _ in 0..2 {
                let Session::Suspended(copy) = Dump::load(&bytes).unwrap().state else {
                    panic!("suspended")
                };
                let mut abandoned = copy.into_repl();
                abandoned
                    .feed_run("pulse = []\npulse = None", vec![], PrintWriter::Stdout)
                    .unwrap();
                assert_eq!(abandoned.heap_entry_count(), baseline, "restored cut {cuts}");
            }
            let mut abandoned = progress.into_repl();
            abandoned
                .feed_run("pulse = []\npulse = None", vec![], PrintWriter::Stdout)
                .unwrap();
            assert_eq!(abandoned.heap_entry_count(), baseline, "original cut {cuts}");
            let Session::Suspended(copy) = Dump::load(&bytes).unwrap().state else {
                panic!("suspended")
            };
            progress = if matches!(*copy, ReplProgress::FunctionCall(_)) {
                let call = copy.into_function_call().unwrap();
                let label = call.args.arg(0).unwrap().as_str().unwrap().to_owned();
                if label == *replies.last().unwrap() {
                    assert_eq!(phase, replies.len() - 1, "parent cleanup before child drain");
                }
                identities.insert(label, call.call_id);
                call.resume_pending(PrintWriter::Stdout).unwrap()
            } else {
                let label = replies[phase];
                phase += 1;
                let pending = copy.into_resolve_futures().unwrap();
                pending
                    .resume(
                        vec![(
                            identities[label],
                            ExtFunctionResult::Return(MontyObject::string(if label == "release" { "r" } else { "ok" })),
                        )],
                        PrintWriter::Stdout,
                    )
                    .unwrap()
            };
        }
        assert_eq!(phase, replies.len());
        let (mut repl, _) = progress.into_complete().unwrap();
        repl.feed_run("pulse = []\npulse = None", vec![], PrintWriter::Stdout)
            .unwrap();
        assert_eq!(repl.heap_entry_count(), baseline, "completion before teardown");
    }
}

/// Group outcome, nested membership, internal interruption and root shutdown roots
/// release before heap teardown, including original and two restored disposals.
#[test]
fn native_async_public_groups_snapshot_ownership() {
    for (name, source, replies, host_cancelled) in [
        (
            "repair7-final-return",
            include_str!(
                "../../../tools/coroutines/cancelled_error/migrated/native_async_groups/cases/repair7-final-return.py"
            ),
            vec!["control", "repeat", "release", "parent-release"],
            false,
        ),
        (
            "repair7-host",
            include_str!("../../../tools/coroutines/native_async_groups/cases/repair7-host.py"),
            vec!["$cancel", "$cancel", "A-release", "B-release", "parent-release"],
            true,
        ),
        (
            "repair7-return",
            include_str!(
                "../../../tools/coroutines/cancelled_error/migrated/native_async_groups/cases/repair7-return.py"
            ),
            vec!["control", "repeat", "release", "parent-release"],
            false,
        ),
        (
            "repair7-caught",
            include_str!(
                "../../../tools/coroutines/cancelled_error/migrated/native_async_groups/cases/repair7-caught.py"
            ),
            vec!["control", "repeat", "release", "parent-release"],
            false,
        ),
        (
            "repair7-body-AB",
            include_str!("../../../tools/coroutines/native_async_groups/cases/repair7-body-AB.py"),
            vec!["A", "B", "A-release", "B-release", "parent-release"],
            false,
        ),
        (
            "repair7-nested-AB",
            include_str!("../../../tools/coroutines/native_async_groups/cases/repair7-nested-AB.py"),
            vec!["A", "B", "A-release", "B-release", "parent-release"],
            false,
        ),
        (
            "repair7-replace-AB",
            include_str!("../../../tools/coroutines/native_async_groups/cases/repair7-replace-AB.py"),
            vec!["A", "B", "A-release", "B-release", "parent-release"],
            false,
        ),
        (
            "repair7-sync-helper-AB",
            include_str!("../../../tools/coroutines/native_async_groups/cases/repair7-sync-helper-AB.py"),
            vec!["A", "B", "A-release", "B-release", "parent-release"],
            false,
        ),
        (
            "repair7-async-helper-AB",
            include_str!("../../../tools/coroutines/native_async_groups/cases/repair7-async-helper-AB.py"),
            vec!["A", "B", "A-release", "B-release", "parent-release"],
            false,
        ),
        (
            "repair7-exit-false-AB",
            include_str!("../../../tools/coroutines/native_async_groups/cases/repair7-exit-false-AB.py"),
            vec!["A", "B", "A-release", "B-release", "parent-release"],
            false,
        ),
        (
            "repair7-exit-raise-AB",
            include_str!("../../../tools/coroutines/native_async_groups/cases/repair7-exit-raise-AB.py"),
            vec!["A", "B", "A-release", "B-release", "parent-release"],
            false,
        ),
        (
            "repair7-cyclic-AB",
            include_str!("../../../tools/coroutines/native_async_groups/cases/repair7-cyclic-AB.py"),
            vec!["A", "B", "A-release", "B-release", "parent-release"],
            false,
        ),
        (
            "repair7-body-BA",
            include_str!("../../../tools/coroutines/native_async_groups/cases/repair7-body-BA.py"),
            vec!["A", "B", "B-release", "A-release", "parent-release"],
            false,
        ),
        (
            "repair7-nested-BA",
            include_str!("../../../tools/coroutines/native_async_groups/cases/repair7-nested-BA.py"),
            vec!["A", "B", "B-release", "A-release", "parent-release"],
            false,
        ),
        (
            "repair7-replace-BA",
            include_str!("../../../tools/coroutines/native_async_groups/cases/repair7-replace-BA.py"),
            vec!["A", "B", "B-release", "A-release", "parent-release"],
            false,
        ),
        (
            "repair7-sync-helper-BA",
            include_str!("../../../tools/coroutines/native_async_groups/cases/repair7-sync-helper-BA.py"),
            vec!["A", "B", "B-release", "A-release", "parent-release"],
            false,
        ),
        (
            "repair7-async-helper-BA",
            include_str!("../../../tools/coroutines/native_async_groups/cases/repair7-async-helper-BA.py"),
            vec!["A", "B", "B-release", "A-release", "parent-release"],
            false,
        ),
        (
            "repair7-exit-false-BA",
            include_str!("../../../tools/coroutines/native_async_groups/cases/repair7-exit-false-BA.py"),
            vec!["A", "B", "B-release", "A-release", "parent-release"],
            false,
        ),
        (
            "repair7-exit-raise-BA",
            include_str!("../../../tools/coroutines/native_async_groups/cases/repair7-exit-raise-BA.py"),
            vec!["A", "B", "B-release", "A-release", "parent-release"],
            false,
        ),
        (
            "repair7-cyclic-BA",
            include_str!("../../../tools/coroutines/native_async_groups/cases/repair7-cyclic-BA.py"),
            vec!["A", "B", "B-release", "A-release", "parent-release"],
            false,
        ),
        (
            "repair7-guest",
            include_str!("../../../tools/coroutines/native_async_groups/cases/repair7-guest.py"),
            vec!["A", "control", "repeat", "A-release", "parent-release"],
            false,
        ),
        (
            "repair7-suppress",
            include_str!(
                "../../../tools/coroutines/cancelled_error/migrated/native_async_groups/cases/repair7-suppress.py"
            ),
            vec!["control", "repeat", "release", "parent-release"],
            false,
        ),
        (
            "retained-AB",
            include_str!("../../../tools/coroutines/native_async_groups/cases/retained-AB.py"),
            vec!["A", "B", "A-release", "B-release", "parent-release"],
            false,
        ),
        (
            "retained-BA",
            include_str!("../../../tools/coroutines/native_async_groups/cases/retained-BA.py"),
            vec!["A", "B", "B-release", "A-release", "parent-release"],
            false,
        ),
        (
            "retained-cyclic",
            include_str!("../../../tools/coroutines/native_async_groups/cases/retained-cyclic.py"),
            vec!["A", "B", "A-release", "B-release", "parent-release"],
            false,
        ),
        (
            "outer-failure-inner-exit",
            include_str!("../../../tools/coroutines/native_async_groups/cases/outer-failure-inner-exit.py"),
            vec!["A", "B-release"],
            false,
        ),
        (
            "depth3-normal-exit",
            include_str!("../../../tools/coroutines/native_async_groups/cases/depth3-normal-exit.py"),
            vec!["A", "B-release", "parent-release"],
            false,
        ),
        (
            "depth3-body",
            include_str!("../../../tools/coroutines/native_async_groups/cases/depth3-body.py"),
            vec!["A", "B-release", "parent-release"],
            false,
        ),
        (
            "depth3-protected",
            include_str!("../../../tools/coroutines/native_async_groups/cases/depth3-protected.py"),
            vec!["body", "A", "B-release", "owner-release", "parent-release"],
            false,
        ),
        (
            "depth3-failure-exit",
            include_str!("../../../tools/coroutines/native_async_groups/cases/depth3-failure-exit.py"),
            vec!["C", "A", "B-release", "parent-release"],
            false,
        ),
        (
            "nested-guest-repeat",
            include_str!(
                "../../../tools/coroutines/cancelled_error/migrated/native_async_groups/cases/nested-guest-repeat.py"
            ),
            vec!["control", "repeat", "B-release", "owner-release"],
            false,
        ),
        (
            "nested-host-repeat",
            include_str!("../../../tools/coroutines/native_async_groups/cases/nested-host-repeat.py"),
            vec!["$cancel", "B-release", "owner-release"],
            true,
        ),
        (
            "guest-natural",
            include_str!("../../../tools/coroutines/native_async_groups/cases/guest-natural.py"),
            vec!["A", "control", "repeat", "A-release", "parent-release"],
            false,
        ),
        (
            "same-owner",
            include_str!("../../../tools/coroutines/native_async_groups/cases/same-owner.py"),
            vec!["B", "after-inner", "A", "parent-release"],
            false,
        ),
        (
            "protected",
            include_str!("../../../tools/coroutines/native_async_groups/cases/protected.py"),
            vec!["body", "A", "C", "child-release", "D", "parent-release"],
            false,
        ),
        (
            "guest-cancel",
            include_str!("../../../tools/coroutines/cancelled_error/migrated/native_async_groups/cases/cancel.py"),
            vec!["control", "repeat", "A-release", "B-release", "owner-release"],
            false,
        ),
        (
            "body-gather",
            include_str!("../../../tools/coroutines/native_async_groups/cases/body-gather.py"),
            vec!["A", "B-release", "K-release", "parent-release"],
            false,
        ),
        (
            "body-joined-child",
            include_str!("../../../tools/coroutines/native_async_groups/cases/body-joined-child.py"),
            vec!["A", "B-release", "K-release", "parent-release"],
            false,
        ),
        (
            "normal",
            include_str!("../../../tools/coroutines/native_async_groups/cases/normal.py"),
            vec!["B", "A", "discarded", "parent-release"],
            false,
        ),
        (
            "failure",
            include_str!("../../../tools/coroutines/native_async_groups/cases/failure.py"),
            vec!["A", "child-release", "parent-release"],
            false,
        ),
        (
            "host-cancel",
            include_str!("../../../tools/coroutines/native_async_groups/cases/host-cancel.py"),
            vec!["$cancel", "A-release", "B-release", "parent-release"],
            true,
        ),
        (
            "root-error",
            include_str!("../../../tools/coroutines/native_async_groups/cases/root-error.py"),
            vec!["control", "child-release"],
            false,
        ),
        (
            "root-shutdown",
            include_str!("../../../tools/coroutines/native_async_groups/cases/root-shutdown.py"),
            vec!["control", "child-release"],
            false,
        ),
    ] {
        let mut repl = new_repl();
        let (definitions, entry) = if name == "root-error" {
            let (definitions, entry) = source.split_once("\ntry:\n    asyncio.run(main())").unwrap();
            (definitions, format!("try:\n    asyncio.run(main()){entry}"))
        } else {
            (
                source.strip_suffix("asyncio.run(main())\n").unwrap(),
                "asyncio.run(main())".to_owned(),
            )
        };
        repl.feed_run(
            definitions,
            vec![("exchange".into(), MontyObject::function("exchange", None))],
            PrintWriter::Stdout,
        )
        .unwrap();
        let baseline = repl.heap_entry_count();
        let mut progress = repl.feed_start(&entry, vec![], PrintWriter::Stdout).unwrap();
        let mut calls = Vec::new();
        for (cut, reply) in replies.into_iter().enumerate() {
            while let ReplProgress::FunctionCall(call) = progress {
                calls.push((call.args.arg(0).unwrap().as_str().unwrap().to_owned(), call.call_id));
                progress = call.resume_pending(PrintWriter::Stdout).unwrap();
            }
            let bytes = dump("groups.py", None, &[], SessionRef::Suspended(&progress)).unwrap();
            for copy in 0..3 {
                let mut disposed = progress.into_resolve_futures().unwrap().into_repl();
                disposed
                    .feed_run("pulse = []\npulse = None", vec![], PrintWriter::Stdout)
                    .unwrap();
                assert_eq!(disposed.heap_entry_count(), baseline, "{name}/{cut}/{copy} disposal");
                let Session::Suspended(restored) = Dump::load(&bytes).unwrap().state else {
                    panic!("pending");
                };
                progress = *restored;
            }
            let results = if reply == "$cancel" {
                vec![]
            } else {
                let id = calls.iter().find(|(label, _)| label == reply).unwrap().1;
                vec![(
                    id,
                    ExtFunctionResult::Return(MontyObject::string(match reply {
                        "A" => "a",
                        "B" => "b",
                        "C" => "c",
                        "D" => "d",
                        _ => "reply",
                    })),
                )]
            };
            progress = progress
                .into_resolve_futures()
                .unwrap()
                .resume_native(
                    results,
                    host_cancelled.then(|| if cut == 0 { "FIRST" } else { "REPEAT" }.into()),
                    PrintWriter::Stdout,
                )
                .unwrap();
        }
        let (mut repl, value) = progress.into_complete().unwrap();
        if host_cancelled {
            repl.feed_run(
                "assert outcome['status'] == 'cancelled'\nassert outcome['reason'] == 'FIRST'\noutcome = None",
                vec![("outcome".into(), value)],
                PrintWriter::Stdout,
            )
            .unwrap();
        } else {
            assert_eq!(value, MontyObject::string("\"ok\""));
        }
        repl.feed_run("pulse = []\npulse = None", vec![], PrintWriter::Stdout)
            .unwrap();
        assert_eq!(repl.heap_entry_count(), baseline, "{name} terminal before teardown");
    }
}

/// Group aggregation retains cyclic exception arguments and cached child identity
/// only while observable; all group roots disappear before the final census.
#[test]
fn native_async_public_groups_cyclic_failure_ownership() {
    let output = checked_run(
        r"
import asyncio
token = []
token.append(token)
async def bad():
    raise ValueError(token)
async def main():
    try:
        async with asyncio.TaskGroup() as group:
            handle = group.create_task(bad())
    except RuntimeError as aggregate:
        assert aggregate.args[0][0] is handle.exception()
        assert aggregate.args[0][0].args[0] is token
asyncio.run(main())
main = bad = None
None
",
    );
    assert_eq!(output.counts["token"], 2, "global plus self edge only");
}

/// New run/barrier roots are disposed independently at every cut, and gone at
/// terminal before the REPL is torn down, including cyclic return/error graphs.
#[test]
fn native_async_run_and_ordinary_cleanup_snapshot_ownership() {
    for (name, source, replies) in [
        (
            "run-cyclic-return",
            include_str!("../../../tools/coroutines/native_async_boundaries/cases/root-cyclic-false.py"),
            vec!["control", "child-release"],
        ),
        (
            "run-cyclic-error",
            include_str!("../../../tools/coroutines/native_async_boundaries/cases/root-cyclic-true.py"),
            vec!["control", "child-release"],
        ),
        (
            "ordinary-finally",
            include_str!("../../../tools/coroutines/native_async_boundaries/cases/body-direct-finally.py"),
            vec!["A", "child-release", "parent-release"],
        ),
        (
            "ordinary-context",
            include_str!("../../../tools/coroutines/native_async_boundaries/cases/body-context.py"),
            vec!["A", "child-release", "parent-release"],
        ),
        (
            "ordinary-helper",
            include_str!("../../../tools/coroutines/native_async_boundaries/cases/body-helper-transfer.py"),
            vec!["A", "child-release", "parent-release"],
        ),
        (
            "ordinary-recovery",
            include_str!("../../../tools/coroutines/native_async_boundaries/cases/body-recovery.py"),
            vec!["A", "recovery", "helper-release", "child-release", "parent-release"],
        ),
        (
            "transferred-direct",
            include_str!("../../../tools/coroutines/native_async_boundaries/cases/body-transferred-direct.py"),
            vec!["A", "B", "child-release", "parent-release"],
        ),
        (
            "run-in-task",
            include_str!("../../../tools/coroutines/native_async_boundaries/cases/run-in-scheduler-task-owned.py"),
            vec!["control", "child-release", "outside"],
        ),
        (
            "run-in-task-nested-group",
            include_str!("../../../tools/coroutines/native_async_boundaries/cases/run-in-nested-task-group-owned.py"),
            vec!["control", "body", "child-release", "outside"],
        ),
        (
            "host-run",
            include_str!("../../../tools/coroutines/native_async_boundaries/cases/root-host.py"),
            vec!["control", "$cancel", "child-release"],
        ),
        (
            "host-cleanup",
            include_str!("../../../tools/coroutines/native_async_boundaries/cases/body-host.py"),
            vec!["A", "$cancel", "child-release", "parent-release"],
        ),
    ] {
        let (definitions, entry) = if let Some(definitions) = source.strip_suffix("asyncio.run(main())\n") {
            (definitions, "asyncio.run(main())".to_owned())
        } else if let Some(definitions) = source.strip_suffix("await entry()\n") {
            (definitions, "await entry()".to_owned())
        } else {
            let (definitions, arg) = source.rsplit_once("\nwrapper(").unwrap();
            (definitions, format!("wrapper({arg}"))
        };
        let mut repl = new_repl();
        repl.feed_run(
            definitions,
            vec![("exchange".into(), MontyObject::function("exchange", None))],
            PrintWriter::Stdout,
        )
        .unwrap();
        let baseline = repl.heap_entry_count();
        let mut progress = repl.feed_start(&entry, vec![], PrintWriter::Stdout).unwrap();
        let mut calls = HashMap::new();
        for (cut, reply) in replies.into_iter().enumerate() {
            while let ReplProgress::FunctionCall(call) = progress {
                calls.insert(call.args.arg(0).unwrap().as_str().unwrap().to_owned(), call.call_id);
                progress = call.resume_pending(PrintWriter::Stdout).unwrap();
            }
            let bytes = dump("boundaries.py", None, &[], SessionRef::Suspended(&progress)).unwrap();
            for copy in 0..3 {
                let mut abandoned = progress.into_resolve_futures().unwrap().into_repl();
                abandoned
                    .feed_run("pulse = []\npulse = None", vec![], PrintWriter::Stdout)
                    .unwrap();
                assert_eq!(abandoned.heap_entry_count(), baseline, "{name}/{cut}/{copy} disposal");
                let Session::Suspended(restored) = Dump::load(&bytes).unwrap().state else {
                    panic!("suspended")
                };
                progress = *restored;
            }
            let results = if reply == "$cancel" {
                Vec::new()
            } else {
                vec![(calls[reply], ExtFunctionResult::Return(MontyObject::string("reply")))]
            };
            progress = progress
                .into_resolve_futures()
                .unwrap()
                .resume_native(
                    results,
                    (name.starts_with("host-") && cut > 0).then(|| if cut == 1 { "FIRST" } else { "REPEAT" }.into()),
                    PrintWriter::Stdout,
                )
                .unwrap();
        }
        let (mut repl, value) = progress.into_complete().unwrap();
        if name.starts_with("host-") {
            repl.feed_run(
                "assert outcome['status'] == 'cancelled'\nassert outcome['reason'] == 'FIRST'\noutcome = None",
                vec![("outcome".into(), value)],
                PrintWriter::Stdout,
            )
            .unwrap();
        } else {
            assert_eq!(value, MontyObject::string("\"ok\""));
        }
        repl.feed_run("pulse = []\npulse = None", vec![], PrintWriter::Stdout)
            .unwrap();
        assert_eq!(repl.heap_entry_count(), baseline, "{name} terminal before teardown");
    }
}

/// Public queues release lazy operations, blocked/reserved deliveries and cyclic
/// values before teardown, including original and two independent restored owners.
#[test]
fn native_async_public_queue_snapshot_ownership() {
    for (name, source, replies) in [
        (
            "reserved-get-cancel",
            include_str!(
                "../../../tools/coroutines/cancelled_error/migrated/native_async_queues/cases/reserved-get-cancel.py"
            ),
            vec!["control", "verified"],
        ),
        (
            "reserved-put-cancel",
            include_str!(
                "../../../tools/coroutines/cancelled_error/migrated/native_async_queues/cases/reserved-put-cancel.py"
            ),
            vec!["control", "verified"],
        ),
        (
            "inspection-lazy",
            include_str!("../../../tools/coroutines/native_async_queues/cases/inspection-lazy.py"),
            vec![
                "before-0",
                "before--2",
                "before-True",
                "before-False",
                "before-1",
                "before-3",
                "before-4294967295",
            ],
        ),
        (
            "unbounded",
            include_str!("../../../tools/coroutines/native_async_queues/cases/unbounded.py"),
            vec!["stored"],
        ),
        (
            "bounded-fifo",
            include_str!("../../../tools/coroutines/native_async_queues/cases/bounded-fifo.py"),
            vec!["full", "empty"],
        ),
        (
            "getters-fifo",
            include_str!("../../../tools/coroutines/native_async_queues/cases/getters-fifo.py"),
            vec!["empty", "done"],
        ),
        (
            "cancel-get",
            include_str!("../../../tools/coroutines/cancelled_error/migrated/native_async_queues/cases/cancel-get.py"),
            vec!["blocked", "done"],
        ),
        (
            "cancel-put",
            include_str!("../../../tools/coroutines/cancelled_error/migrated/native_async_queues/cases/cancel-put.py"),
            vec!["blocked", "done"],
        ),
        (
            "delivered-cancel",
            include_str!(
                "../../../tools/coroutines/cancelled_error/migrated/native_async_queues/cases/delivered-cancel.py"
            ),
            vec!["control"],
        ),
        (
            "group-failure",
            include_str!("../../../tools/coroutines/native_async_queues/cases/group-failure.py"),
            vec!["fail", "child-release", "parent-release"],
        ),
        (
            "ordinary-cleanup",
            include_str!("../../../tools/coroutines/native_async_queues/cases/ordinary-cleanup.py"),
            vec!["fail", "child-release", "parent-release"],
        ),
        (
            "run-shutdown",
            include_str!("../../../tools/coroutines/native_async_queues/cases/run-shutdown.py"),
            vec!["control", "child-release"],
        ),
        (
            "protected-cleanup",
            include_str!("../../../tools/coroutines/native_async_queues/cases/protected-cleanup.py"),
            vec!["body", "release", "parent-release"],
        ),
        (
            "host-cleanup",
            include_str!("../../../tools/coroutines/native_async_queues/cases/host-cleanup.py"),
            vec!["$cancel", "$cancel", "release", "parent-release"],
        ),
        (
            "host-group",
            include_str!("../../../tools/coroutines/native_async_queues/cases/host-group.py"),
            vec!["$cancel", "$cancel", "child-release", "parent-release"],
        ),
        (
            "misuse-reawait",
            include_str!("../../../tools/coroutines/native_async_queues/cases/misuse-reawait.py"),
            vec!["checked"],
        ),
        (
            "reservations",
            include_str!("../../../tools/coroutines/native_async_queues/cases/reservations.py"),
            vec!["put-reserved", "get-reserved"],
        ),
        (
            "cyclic-blocked-put",
            include_str!(
                "../../../tools/coroutines/cancelled_error/migrated/native_async_queues/cases/cyclic-blocked-put.py"
            ),
            vec!["blocked", "delivered"],
        ),
    ] {
        let definitions = source.strip_suffix("asyncio.run(main())\n").unwrap();
        let mut repl = new_repl();
        repl.feed_run(
            definitions,
            vec![("exchange".into(), MontyObject::function("exchange", None))],
            PrintWriter::Disabled,
        )
        .unwrap();
        let baseline = repl.heap_entry_count();
        for repetition in 0..4 {
            let mut progress = repl
                .feed_start("asyncio.run(main())", vec![], PrintWriter::Disabled)
                .unwrap();
            let mut calls = HashMap::new();
            let mut host_cancel = false;
            for (cut, reply) in replies.iter().enumerate() {
                loop {
                    // FunctionCall cuts include ready but unconsumed reservations.
                    let bytes = dump("queues.py", None, &[], SessionRef::Suspended(&progress)).unwrap();
                    for copy in 0..3 {
                        let mut abandoned = progress.into_repl();
                        abandoned
                            .feed_run("pulse = []\npulse = None", vec![], PrintWriter::Disabled)
                            .unwrap();
                        assert_eq!(
                            abandoned.heap_entry_count(),
                            baseline,
                            "{name}/{repetition}/{cut}/{copy} abandonment"
                        );
                        let Session::Suspended(restored) = Dump::load(&bytes).unwrap().state else {
                            panic!("suspended")
                        };
                        progress = *restored;
                    }
                    if let ReplProgress::FunctionCall(call) = progress {
                        calls.insert(call.args.arg(0).unwrap().as_str().unwrap().to_owned(), call.call_id);
                        progress = call.resume_pending(PrintWriter::Disabled).unwrap();
                    } else {
                        break;
                    }
                }
                let results = if *reply == "$cancel" {
                    host_cancel = true;
                    vec![]
                } else {
                    let mut results = vec![(calls[*reply], ExtFunctionResult::Return(MontyObject::string("reply")))];
                    if name.starts_with("reserved-") && *reply == "control" {
                        results.push((
                            calls["cancelled-reservation"],
                            ExtFunctionResult::Return(MontyObject::string("reply")),
                        ));
                    }
                    results
                };
                progress = progress
                    .into_resolve_futures()
                    .unwrap()
                    .resume_native(results, host_cancel.then(|| "FIRST".into()), PrintWriter::Disabled)
                    .unwrap();
            }
            let (mut completed, value) = progress.into_complete().unwrap();
            if host_cancel {
                completed
                    .feed_run(
                        "assert outcome['status'] == 'cancelled'\nassert outcome['reason'] == 'FIRST'\noutcome = None",
                        vec![("outcome".into(), value)],
                        PrintWriter::Disabled,
                    )
                    .unwrap();
            } else {
                assert_eq!(value, MontyObject::string("\"ok\""));
            }
            completed
                .feed_run("pulse = []\npulse = None", vec![], PrintWriter::Disabled)
                .unwrap();
            assert_eq!(
                completed.heap_entry_count(),
                baseline,
                "{name}/{repetition} terminal before teardown"
            );
            repl = completed;
        }
    }
}

/// Supplied start_soon retains arguments only through existing coroutine/group
/// roots; rejected closed/foreign calls do not invoke an authored callable.
#[test]
fn native_async_anyio_start_soon_live_ownership() {
    let output = checked_run(
        r"
import asyncio
import anyio
token = []
token.append(token)
calls = []
async def child(arg):
    assert arg is token
def forbidden(arg):
    calls.append(arg)
    return child(arg)
async def foreign(group):
    try:
        group.start_soon(forbidden, token)
        assert False
    except RuntimeError:
        pass
async def main():
    group = anyio.create_task_group()
    method = group.start_soon
    try:
        method(forbidden, token)
        assert False
    except RuntimeError:
        pass
    async with group:
        assert method(child, token) is None
        assert await asyncio.create_task(foreign(group)) is None
    try:
        method(forbidden, token)
        assert False
    except RuntimeError:
        pass
    assert calls == []
    with anyio.CancelScope(shield=True) as scope:
        assert scope.shield and not scope.cancel_called and not scope.cancelled_caught
asyncio.run(main())
main = child = forbidden = foreign = calls = None
None
",
    );
    assert_eq!(output.counts["token"], 2, "global plus self edge only");
}

/// Saved start_soon methods, cyclic coroutine arguments and group outcomes
/// survive genuine snapshots and release their roots before interpreter teardown.
/// The synchronous admission frame has already returned at these cuts.
#[test]
fn native_async_anyio_snapshot_ownership() {
    let mut repl = new_repl();
    repl.feed_run(
        r"
import asyncio
import anyio
async def child(token):
    assert token[0] is token
    await exchange('child')
async def main():
    token = []
    token.append(token)
    async with anyio.create_task_group() as group:
        start = group.start_soon
        start(child, token)
    await exchange('last')
",
        vec![("exchange".into(), MontyObject::function("exchange", None))],
        PrintWriter::Disabled,
    )
    .unwrap();
    let baseline = repl.heap_entry_count();
    let mut progress = repl
        .feed_start("asyncio.run(main())", vec![], PrintWriter::Disabled)
        .unwrap();
    let mut cuts = 0;
    while !matches!(progress, ReplProgress::Complete { .. }) {
        cuts += 1;
        assert!(cuts < 8, "bounded AnyIO trace");
        let bytes = dump("anyio.py", None, &[], SessionRef::Suspended(&progress)).unwrap();
        for _ in 0..2 {
            let Session::Suspended(copy) = Dump::load(&bytes).unwrap().state else {
                panic!("suspended")
            };
            let mut abandoned = copy.into_repl();
            abandoned
                .feed_run("pulse = []\npulse = None", vec![], PrintWriter::Disabled)
                .unwrap();
            assert_eq!(abandoned.heap_entry_count(), baseline, "restored cut {cuts}");
        }
        let mut abandoned = progress.into_repl();
        abandoned
            .feed_run("pulse = []\npulse = None", vec![], PrintWriter::Disabled)
            .unwrap();
        assert_eq!(abandoned.heap_entry_count(), baseline, "original cut {cuts}");
        let Session::Suspended(copy) = Dump::load(&bytes).unwrap().state else {
            panic!("suspended")
        };
        progress = if matches!(*copy, ReplProgress::FunctionCall(_)) {
            copy.into_function_call()
                .unwrap()
                .resume_pending(PrintWriter::Disabled)
                .unwrap()
        } else {
            let pending = copy.into_resolve_futures().unwrap();
            let ids = pending.pending_call_ids().to_vec();
            pending
                .resume(
                    ids.into_iter()
                        .map(|id| (id, ExtFunctionResult::Return(MontyObject::none())))
                        .collect(),
                    PrintWriter::Disabled,
                )
                .unwrap()
        };
    }
    let (mut repl, _) = progress.into_complete().unwrap();
    repl.feed_run("pulse = []\npulse = None", vec![], PrintWriter::Disabled)
        .unwrap();
    assert_eq!(repl.heap_entry_count(), baseline, "completed before teardown");
}

/// Repeated level delivery and retained lexical carriers release all task roots
/// before interpreter teardown, including cyclic exception graphs.
#[test]
fn native_async_anyio_scope_live_ownership() {
    let output = checked_run(
        r"
import asyncio
import anyio
token = []
token.append(token)
error = ValueError(token)
token.append(error)
async def checkpoint():
    assert False
async def main():
    for i in range(64):
        with anyio.CancelScope() as scope:
            scope.cancel()
            for j in range(2):
                try:
                    await checkpoint()
                    assert False
                except asyncio.CancelledError as signal:
                    token.append(scope)
                    token.append(signal)
            with anyio.CancelScope(shield=True):
                assert not scope.cancelled_caught
        assert not scope.cancelled_caught
    token.clear()
    token.append(token)
    token.append(error)
asyncio.run(main())
main = checkpoint = error = None
None
",
    );
    assert_eq!(output.counts["token"], 3, "global, self, cyclic error argument only");
}

/// Original abandonment and two fresh restored owners at every real effect cut.
/// Counts are measured after collection while the interpreter remains live.
#[test]
fn native_async_anyio_scope_snapshot_ownership() {
    let mut repl = new_repl();
    repl.feed_run(
        r"
import asyncio
import anyio
async def main():
    for i in range(3):
        token = []
        token.append(token)
        with anyio.CancelScope() as scope:
            scope.cancel()
            try:
                await exchange('forbidden')
            except asyncio.CancelledError as signal:
                token.append(signal)
                token.append(scope)
            with anyio.CancelScope(shield=True):
                await exchange('cleanup')
        assert not scope.cancelled_caught
        token = scope = None
    await exchange('last')
",
        vec![("exchange".into(), MontyObject::function("exchange", None))],
        PrintWriter::Disabled,
    )
    .unwrap();
    let baseline = repl.heap_entry_count();
    let mut progress = repl
        .feed_start("asyncio.run(main())", vec![], PrintWriter::Disabled)
        .unwrap();
    let mut cuts = 0;
    while !matches!(progress, ReplProgress::Complete { .. }) {
        cuts += 1;
        assert!(cuts <= 8);
        let bytes = dump("scope.py", None, &[], SessionRef::Suspended(&progress)).unwrap();
        assert_eq!(&bytes[..8], b"MONTY\0\x24\x00");
        for _ in 0..2 {
            let Session::Suspended(copy) = Dump::load(&bytes).unwrap().state else {
                panic!("suspended")
            };
            let mut abandoned = copy.into_repl();
            abandoned
                .feed_run("pulse = []\npulse = None", vec![], PrintWriter::Disabled)
                .unwrap();
            assert_eq!(abandoned.heap_entry_count(), baseline, "restored scope cut {cuts}");
        }
        let mut abandoned = progress.into_repl();
        abandoned
            .feed_run("pulse = []\npulse = None", vec![], PrintWriter::Disabled)
            .unwrap();
        assert_eq!(abandoned.heap_entry_count(), baseline, "original scope cut {cuts}");
        let Session::Suspended(copy) = Dump::load(&bytes).unwrap().state else {
            panic!("suspended")
        };
        progress = if matches!(*copy, ReplProgress::FunctionCall(_)) {
            let call = copy.into_function_call().unwrap();
            assert_ne!(call.args.arg(0).unwrap().as_str(), Some("forbidden"));
            call.resume_pending(PrintWriter::Disabled).unwrap()
        } else {
            let pending = copy.into_resolve_futures().unwrap();
            let ids = pending.pending_call_ids().to_vec();
            pending
                .resume(
                    ids.into_iter()
                        .map(|id| (id, ExtFunctionResult::Return(MontyObject::none())))
                        .collect(),
                    PrintWriter::Disabled,
                )
                .unwrap()
        };
    }
    assert_eq!(cuts, 8);
    let (mut repl, _) = progress.into_complete().unwrap();
    repl.feed_run("pulse = []\npulse = None", vec![], PrintWriter::Disabled)
        .unwrap();
    assert_eq!(repl.heap_entry_count(), baseline, "completed scope history is bounded");
}

/// Cancellation/drain census includes actual cyclic cleanup errors, inherited
/// scopes, nested groups and internal queues, at every public suspension cut.
#[test]
fn native_async_anyio_group_scope_snapshot_ownership() {
    for source in [
        include_str!("../../../tools/coroutines/anyio_scopes/cases/group-cancel.py"),
        include_str!("../../../tools/coroutines/anyio_scopes/cases/queue-cancel.py"),
        include_str!("../../../tools/coroutines/anyio_scopes/cases/nested-group.py"),
        include_str!("../../../tools/coroutines/anyio_scopes/cases/cleanup-error.py"),
        include_str!("../../../tools/coroutines/anyio_scopes/cases/group-misuse.py"),
        include_str!("../../../tools/coroutines/anyio_scopes/cases/group-owner-isolation.py"),
        include_str!("../../../tools/coroutines/anyio_scopes/cases/group-ancestor-shield.py"),
        include_str!("../../../tools/coroutines/cancelled_error/migrated/anyio_scopes/cases/inherited-shield-exit.py"),
    ] {
        let definitions = source.strip_suffix("asyncio.run(main())\n").expect("frozen entry");
        let mut repl = new_repl();
        repl.feed_run(
            definitions,
            vec![("exchange".into(), MontyObject::function("exchange", None))],
            PrintWriter::Disabled,
        )
        .unwrap();
        let baseline = repl.heap_entry_count();
        let mut progress = repl
            .feed_start("asyncio.run(main())", vec![], PrintWriter::Disabled)
            .unwrap();
        let mut cuts = 0;
        while !matches!(progress, ReplProgress::Complete { .. }) {
            cuts += 1;
            assert!(cuts < 16);
            let bytes = dump("group-scope.py", None, &[], SessionRef::Suspended(&progress)).unwrap();
            for _ in 0..2 {
                let Session::Suspended(copy) = Dump::load(&bytes).unwrap().state else {
                    panic!("suspended")
                };
                let mut abandoned = copy.into_repl();
                abandoned
                    .feed_run("pulse = []\npulse = None", vec![], PrintWriter::Disabled)
                    .unwrap();
                assert_eq!(
                    abandoned.heap_entry_count(),
                    baseline,
                    "restored group cut {cuts}: {source}"
                );
            }
            let mut abandoned = progress.into_repl();
            abandoned
                .feed_run("pulse = []\npulse = None", vec![], PrintWriter::Disabled)
                .unwrap();
            assert_eq!(
                abandoned.heap_entry_count(),
                baseline,
                "original group cut {cuts}: {source}"
            );
            let Session::Suspended(copy) = Dump::load(&bytes).unwrap().state else {
                panic!("suspended")
            };
            progress = if matches!(*copy, ReplProgress::FunctionCall(_)) {
                copy.into_function_call()
                    .unwrap()
                    .resume_pending(PrintWriter::Disabled)
                    .unwrap()
            } else {
                let pending = copy.into_resolve_futures().unwrap();
                let ids = pending.pending_call_ids().to_vec();
                // Initial owner control settles before the blocked child. Later
                // stages each have only their legitimate cleanup/after wait.
                let selected = *ids.iter().min().unwrap();
                pending
                    .resume(
                        vec![(selected, ExtFunctionResult::Return(MontyObject::string("ok")))],
                        PrintWriter::Disabled,
                    )
                    .unwrap()
            };
        }
        let (mut repl, _) = progress.into_complete().unwrap();
        repl.feed_run("pulse = []\npulse = None", vec![], PrintWriter::Disabled)
            .unwrap();
        assert_eq!(
            repl.heap_entry_count(),
            baseline,
            "completed group scope history: {source}"
        );
    }
}

/// Even unmatched manual scope entries cannot become completed-run history.
#[test]
fn native_async_anyio_unclosed_scope_run_ownership() {
    let mut repl = new_repl();
    repl.feed_run(
        r"
import asyncio
import anyio
async def main():
    scope = anyio.CancelScope()
    scope.__enter__()
    scope.cancel()
",
        vec![],
        PrintWriter::Disabled,
    )
    .unwrap();
    let baseline = repl.heap_entry_count();
    repl.feed_run(
        "for i in range(32):\n    asyncio.run(main())\npulse = []\npulse = None",
        vec![],
        PrintWriter::Disabled,
    )
    .unwrap();
    assert_eq!(repl.heap_entry_count(), baseline, "no completed-run scope roots");
}

/// Completed group-valued Tasks checkpoint normally and retain balanced roots
/// through original/restored abandonment, aliases, shields and pending joins.
#[test]
fn native_async_anyio_group_entry_role_snapshot_ownership() {
    for source in [
        include_str!("../../../tools/coroutines/anyio_scopes_repair2/cases/group.py"),
        include_str!("../../../tools/coroutines/anyio_scopes_repair2/cases/native-group.py"),
        include_str!("../../../tools/coroutines/anyio_scopes_repair2/cases/list.py"),
        include_str!("../../../tools/coroutines/anyio_scopes_repair2/cases/scope.py"),
        include_str!(
            "../../../tools/coroutines/cancelled_error/migrated/anyio_scopes_repair2/cases/alias-shield-group.py"
        ),
        include_str!("../../../tools/coroutines/anyio_scopes_repair2/cases/pending-group.py"),
        include_str!(
            "../../../tools/coroutines/cancelled_error/migrated/anyio_scopes_repair2/cases/alias-shield-native-group.py"
        ),
        include_str!("../../../tools/coroutines/anyio_scopes_repair2/cases/pending-native-group.py"),
    ] {
        let definitions = source.strip_suffix("asyncio.run(main())\n").expect("frozen entry");
        let mut repl = new_repl();
        repl.feed_run(
            definitions,
            vec![("exchange".into(), MontyObject::function("exchange", None))],
            PrintWriter::Disabled,
        )
        .unwrap();
        let baseline = repl.heap_entry_count();
        let mut progress = repl
            .feed_start("asyncio.run(main())", vec![], PrintWriter::Disabled)
            .unwrap();
        let mut cuts = 0;
        while !matches!(progress, ReplProgress::Complete { .. }) {
            cuts += 1;
            assert!(cuts < 16);
            let bytes = dump("group-scope.py", None, &[], SessionRef::Suspended(&progress)).unwrap();
            for _ in 0..2 {
                let Session::Suspended(copy) = Dump::load(&bytes).unwrap().state else {
                    panic!("suspended")
                };
                let mut abandoned = copy.into_repl();
                abandoned
                    .feed_run("pulse = []\npulse = None", vec![], PrintWriter::Disabled)
                    .unwrap();
                assert_eq!(
                    abandoned.heap_entry_count(),
                    baseline,
                    "restored group cut {cuts}: {source}"
                );
            }
            let mut abandoned = progress.into_repl();
            abandoned
                .feed_run("pulse = []\npulse = None", vec![], PrintWriter::Disabled)
                .unwrap();
            assert_eq!(
                abandoned.heap_entry_count(),
                baseline,
                "original group cut {cuts}: {source}"
            );
            let Session::Suspended(copy) = Dump::load(&bytes).unwrap().state else {
                panic!("suspended")
            };
            progress = if matches!(*copy, ReplProgress::FunctionCall(_)) {
                copy.into_function_call()
                    .unwrap()
                    .resume_pending(PrintWriter::Disabled)
                    .unwrap()
            } else {
                let pending = copy.into_resolve_futures().unwrap();
                let ids = pending.pending_call_ids().to_vec();
                // Initial owner control settles before the blocked child. Later
                // stages each have only their legitimate cleanup/after wait.
                let selected = *ids.iter().min().unwrap();
                pending
                    .resume(
                        vec![(selected, ExtFunctionResult::Return(MontyObject::string("ok")))],
                        PrintWriter::Disabled,
                    )
                    .unwrap()
            };
        }
        let (mut repl, output) = progress.into_complete().unwrap();
        assert_eq!(output, MontyObject::string("\"ok\""));
        repl.feed_run("pulse = []\npulse = None", vec![], PrintWriter::Disabled)
            .unwrap();
        assert_eq!(
            repl.heap_entry_count(),
            baseline,
            "completed group scope history: {source}"
        );
    }
}

/// Supplied capabilities are ordinary native coroutines, including direct awaits
/// and public Tasks. Restores keep their compiled implementation and host wait.
#[test]
fn public_sandbox_coroutines_restore_without_dialect() {
    for capability in ["read_file('a.txt')", "ask({'message': 'question'})"] {
        for entry in ["direct", "task"] {
            let source = format!(
                "import asyncio\nfrom sandbox import read_file, ask\nasync def main():\n    call = {capability}\n    return await {}\nasyncio.run(main())",
                if entry == "task" {
                    "asyncio.create_task(call)"
                } else {
                    "call"
                }
            );
            let repl = new_repl();
            let call = repl
                .feed_start(&source, vec![], PrintWriter::Disabled)
                .unwrap()
                .into_function_call()
                .unwrap();
            assert_eq!(call.function_name, "sandbox.exchange");
            let progress = call.resume_pending(PrintWriter::Disabled).unwrap();
            let bytes = dump("sandbox.py", None, &[], SessionRef::Suspended(&progress)).unwrap();
            for _ in 0..2 {
                let Session::Suspended(copy) = Dump::load(&bytes).unwrap().state else {
                    panic!("expected genuine suspended snapshot")
                };
                let pending = copy.into_resolve_futures().unwrap();
                let id = pending.pending_call_ids()[0];
                let complete = pending
                    .resume_native(
                        vec![(id, ExtFunctionResult::Return(MontyObject::string("reply")))],
                        None,
                        PrintWriter::Disabled,
                    )
                    .unwrap();
                let ReplProgress::Complete { value, .. } = complete else {
                    panic!("expected ordinary public return")
                };
                assert_eq!(value.as_ref().as_str(), Some("reply"));
            }
            let mut abandoned = progress.into_resolve_futures().unwrap().into_repl();
            abandoned
                .feed_run("pulse = []\npulse = None", vec![], PrintWriter::Disabled)
                .unwrap();
        }
    }
}

#[test]
fn public_cancelled_error_class_and_guest_scopes() {
    let mut repl = new_repl();
    let output = repl
        .feed_run(
            r"
import asyncio
import anyio
assert anyio.get_cancelled_exc_class() is asyncio.CancelledError
assert isinstance(asyncio.CancelledError('stop'), BaseException)
assert not isinstance(asyncio.CancelledError('stop'), Exception)
assert not isinstance(asyncio.CancelledError('stop'), ValueError)
async def main():
    seen = []
    with anyio.CancelScope() as scope:
        scope.cancel()
        try:
            await asyncio.gather()
        except ValueError:
            seen.append('wrong-value')
        except Exception:
            seen.append('wrong-exception')
        except asyncio.CancelledError:
            seen.append('cancel')
    try:
        raise ValueError('ordinary')
    except asyncio.CancelledError:
        seen.append('wrong-cancel')
    except ValueError:
        seen.append('value')
    return seen
asyncio.run(main())
",
            vec![],
            PrintWriter::Disabled,
        )
        .unwrap();
    assert_eq!(
        output,
        MontyObject::list(vec![MontyObject::string("cancel"), MontyObject::string("value")])
    );
}
