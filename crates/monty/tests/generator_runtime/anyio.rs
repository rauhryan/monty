//! AnyIO groups, cancellation scopes and shielding across live and restored execution.

use monty::{Dump, ReplProgress, Session, SessionRef, dump};
use monty_types::{ExtFunctionResult, MontyObject, PrintWriter};

use super::support::{checked_run, new_repl};

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
        assert_eq!(&bytes[..6], b"MONTY\0");
        assert_eq!(&bytes[6..8], &monty::DUMP_VERSION.to_le_bytes());
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
        include_str!("../fixtures/concurrency/cancel_scopes/group-cancel.py"),
        include_str!("../fixtures/concurrency/cancel_scopes/queue-cancel.py"),
        include_str!("../fixtures/concurrency/cancel_scopes/nested-group.py"),
        include_str!("../fixtures/concurrency/cancel_scopes/cleanup-error.py"),
        include_str!("../fixtures/concurrency/cancel_scopes/group-misuse.py"),
        include_str!("../fixtures/concurrency/cancel_scopes/group-owner-isolation.py"),
        include_str!("../fixtures/concurrency/cancel_scopes/group-ancestor-shield.py"),
        include_str!("../fixtures/concurrency/cancel_scopes/inherited-shield-exit.py"),
    ] {
        let definitions = source.strip_suffix("asyncio.run(main())\n").expect("frozen entry");
        let mut repl = new_repl();
        repl.feed_run(
            definitions,
            vec![("exchange".into(), MontyObject::function("exchange", None))],
            PrintWriter::Disabled,
        )
        .unwrap();
        let retained_roots = source == include_str!("../fixtures/concurrency/cancel_scopes/cleanup-error.py");
        // This fixture deliberately roots an exception/args/list cycle globally.
        // Upstream chaining adds a reachable cancellation context to that object.
        // Release the three original objects as well before checking abandonment;
        // every chained exception must then be collected, not excused by a count.
        let baseline = repl.heap_entry_count() - if retained_roots { 3 } else { 0 };
        let collect = if retained_roots {
            "assert error.args[0] is cycle\nassert error.__context__ is None or isinstance(error.__context__, asyncio.CancelledError)\nerror = cycle = None\npulse = []\npulse = None"
        } else {
            "pulse = []\npulse = None"
        };
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
                abandoned.feed_run(collect, vec![], PrintWriter::Disabled).unwrap();
                assert_eq!(
                    abandoned.heap_entry_count(),
                    baseline,
                    "restored group cut {cuts}: {source}"
                );
            }
            let mut abandoned = progress.into_repl();
            abandoned.feed_run(collect, vec![], PrintWriter::Disabled).unwrap();
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
        repl.feed_run(collect, vec![], PrintWriter::Disabled).unwrap();
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
        include_str!("../fixtures/concurrency/scope_checkpoints/group.py"),
        include_str!("../fixtures/concurrency/scope_checkpoints/native-group.py"),
        include_str!("../fixtures/concurrency/scope_checkpoints/list.py"),
        include_str!("../fixtures/concurrency/scope_checkpoints/scope.py"),
        include_str!("../fixtures/concurrency/scope_checkpoints/alias-shield-group.py"),
        include_str!("../fixtures/concurrency/scope_checkpoints/pending-group.py"),
        include_str!("../fixtures/concurrency/scope_checkpoints/alias-shield-native-group.py"),
        include_str!("../fixtures/concurrency/scope_checkpoints/pending-native-group.py"),
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
