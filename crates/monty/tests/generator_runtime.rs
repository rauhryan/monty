//! Focused generator ownership checks run before interpreter/heap teardown.
#![cfg(feature = "ref-count-return")]

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

/// Version 14 omitted generator fields; resume the real old host-cut fixture.
#[test]
fn previous_snapshot_resumes() {
    let Session::Suspended(progress) = Dump::load(include_bytes!("fixtures/dump_v14.bin")).unwrap().state else {
        panic!("legacy cut")
    };
    let (_, value) = progress
        .into_function_call()
        .unwrap()
        .resume(MontyObject::int(41), PrintWriter::Stdout)
        .unwrap()
        .into_complete()
        .unwrap();
    assert_eq!(value, MontyObject::int(42));
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
