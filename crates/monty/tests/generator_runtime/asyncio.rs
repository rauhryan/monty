//! Asyncio tasks, groups, queues and host calls across completion, restore and disposal.

use std::collections::HashMap;

use monty::{Dump, ReplProgress, Session, SessionRef, dump};
use monty_types::{ExcType, ExtFunctionResult, MontyObject, PrintWriter};

use super::support::{checked_run, new_repl};

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
            include_str!("../fixtures/concurrency/tasks/cancel.py"),
            vec!["control", "again", "release", "waiter-release"],
        ),
        (
            include_str!("../fixtures/concurrency/tasks/cancel-gather.py"),
            vec!["control", "A-release", "B-release", "owner-release"],
        ),
        (
            include_str!("../fixtures/concurrency/tasks/cancel-gather-wrapped.py"),
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
            include_str!("../fixtures/concurrency/task_groups/repair7-final-return.py"),
            vec!["control", "repeat", "release", "parent-release"],
            false,
        ),
        (
            "repair7-host",
            include_str!("../fixtures/concurrency/task_groups/repair7-host.py"),
            vec!["$cancel", "$cancel", "A-release", "B-release", "parent-release"],
            true,
        ),
        (
            "repair7-return",
            include_str!("../fixtures/concurrency/task_groups/repair7-return.py"),
            vec!["control", "repeat", "release", "parent-release"],
            false,
        ),
        (
            "repair7-caught",
            include_str!("../fixtures/concurrency/task_groups/repair7-caught.py"),
            vec!["control", "repeat", "release", "parent-release"],
            false,
        ),
        (
            "repair7-body-AB",
            include_str!("../fixtures/concurrency/task_groups/repair7-body-AB.py"),
            vec!["A", "B", "A-release", "B-release", "parent-release"],
            false,
        ),
        (
            "repair7-nested-AB",
            include_str!("../fixtures/concurrency/task_groups/repair7-nested-AB.py"),
            vec!["A", "B", "A-release", "B-release", "parent-release"],
            false,
        ),
        (
            "repair7-replace-AB",
            include_str!("../fixtures/concurrency/task_groups/repair7-replace-AB.py"),
            vec!["A", "B", "A-release", "B-release", "parent-release"],
            false,
        ),
        (
            "repair7-sync-helper-AB",
            include_str!("../fixtures/concurrency/task_groups/repair7-sync-helper-AB.py"),
            vec!["A", "B", "A-release", "B-release", "parent-release"],
            false,
        ),
        (
            "repair7-async-helper-AB",
            include_str!("../fixtures/concurrency/task_groups/repair7-async-helper-AB.py"),
            vec!["A", "B", "A-release", "B-release", "parent-release"],
            false,
        ),
        (
            "repair7-exit-false-AB",
            include_str!("../fixtures/concurrency/task_groups/repair7-exit-false-AB.py"),
            vec!["A", "B", "A-release", "B-release", "parent-release"],
            false,
        ),
        (
            "repair7-exit-raise-AB",
            include_str!("../fixtures/concurrency/task_groups/repair7-exit-raise-AB.py"),
            vec!["A", "B", "A-release", "B-release", "parent-release"],
            false,
        ),
        (
            "repair7-cyclic-AB",
            include_str!("../fixtures/concurrency/task_groups/repair7-cyclic-AB.py"),
            vec!["A", "B", "A-release", "B-release", "parent-release"],
            false,
        ),
        (
            "repair7-body-BA",
            include_str!("../fixtures/concurrency/task_groups/repair7-body-BA.py"),
            vec!["A", "B", "B-release", "A-release", "parent-release"],
            false,
        ),
        (
            "repair7-nested-BA",
            include_str!("../fixtures/concurrency/task_groups/repair7-nested-BA.py"),
            vec!["A", "B", "B-release", "A-release", "parent-release"],
            false,
        ),
        (
            "repair7-replace-BA",
            include_str!("../fixtures/concurrency/task_groups/repair7-replace-BA.py"),
            vec!["A", "B", "B-release", "A-release", "parent-release"],
            false,
        ),
        (
            "repair7-sync-helper-BA",
            include_str!("../fixtures/concurrency/task_groups/repair7-sync-helper-BA.py"),
            vec!["A", "B", "B-release", "A-release", "parent-release"],
            false,
        ),
        (
            "repair7-async-helper-BA",
            include_str!("../fixtures/concurrency/task_groups/repair7-async-helper-BA.py"),
            vec!["A", "B", "B-release", "A-release", "parent-release"],
            false,
        ),
        (
            "repair7-exit-false-BA",
            include_str!("../fixtures/concurrency/task_groups/repair7-exit-false-BA.py"),
            vec!["A", "B", "B-release", "A-release", "parent-release"],
            false,
        ),
        (
            "repair7-exit-raise-BA",
            include_str!("../fixtures/concurrency/task_groups/repair7-exit-raise-BA.py"),
            vec!["A", "B", "B-release", "A-release", "parent-release"],
            false,
        ),
        (
            "repair7-cyclic-BA",
            include_str!("../fixtures/concurrency/task_groups/repair7-cyclic-BA.py"),
            vec!["A", "B", "B-release", "A-release", "parent-release"],
            false,
        ),
        (
            "repair7-guest",
            include_str!("../fixtures/concurrency/task_groups/repair7-guest.py"),
            vec!["A", "control", "repeat", "A-release", "parent-release"],
            false,
        ),
        (
            "repair7-suppress",
            include_str!("../fixtures/concurrency/task_groups/repair7-suppress.py"),
            vec!["control", "repeat", "release", "parent-release"],
            false,
        ),
        (
            "retained-AB",
            include_str!("../fixtures/concurrency/task_groups/retained-AB.py"),
            vec!["A", "B", "A-release", "B-release", "parent-release"],
            false,
        ),
        (
            "retained-BA",
            include_str!("../fixtures/concurrency/task_groups/retained-BA.py"),
            vec!["A", "B", "B-release", "A-release", "parent-release"],
            false,
        ),
        (
            "retained-cyclic",
            include_str!("../fixtures/concurrency/task_groups/retained-cyclic.py"),
            vec!["A", "B", "A-release", "B-release", "parent-release"],
            false,
        ),
        (
            "outer-failure-inner-exit",
            include_str!("../fixtures/concurrency/task_groups/outer-failure-inner-exit.py"),
            vec!["A", "B-release"],
            false,
        ),
        (
            "depth3-normal-exit",
            include_str!("../fixtures/concurrency/task_groups/depth3-normal-exit.py"),
            vec!["A", "B-release", "parent-release"],
            false,
        ),
        (
            "depth3-body",
            include_str!("../fixtures/concurrency/task_groups/depth3-body.py"),
            vec!["A", "B-release", "parent-release"],
            false,
        ),
        (
            "depth3-protected",
            include_str!("../fixtures/concurrency/task_groups/depth3-protected.py"),
            vec!["body", "A", "B-release", "owner-release", "parent-release"],
            false,
        ),
        (
            "depth3-failure-exit",
            include_str!("../fixtures/concurrency/task_groups/depth3-failure-exit.py"),
            vec!["C", "A", "B-release", "parent-release"],
            false,
        ),
        (
            "nested-guest-repeat",
            include_str!("../fixtures/concurrency/task_groups/nested-guest-repeat.py"),
            vec!["control", "repeat", "B-release", "owner-release"],
            false,
        ),
        (
            "nested-host-repeat",
            include_str!("../fixtures/concurrency/task_groups/nested-host-repeat.py"),
            vec!["$cancel", "B-release", "owner-release"],
            true,
        ),
        (
            "guest-natural",
            include_str!("../fixtures/concurrency/task_groups/guest-natural.py"),
            vec!["A", "control", "repeat", "A-release", "parent-release"],
            false,
        ),
        (
            "same-owner",
            include_str!("../fixtures/concurrency/task_groups/same-owner.py"),
            vec!["B", "after-inner", "A", "parent-release"],
            false,
        ),
        (
            "protected",
            include_str!("../fixtures/concurrency/task_groups/protected.py"),
            vec!["body", "A", "C", "child-release", "D", "parent-release"],
            false,
        ),
        (
            "guest-cancel",
            include_str!("../fixtures/concurrency/task_groups/cancel.py"),
            vec!["control", "repeat", "A-release", "B-release", "owner-release"],
            false,
        ),
        (
            "body-gather",
            include_str!("../fixtures/concurrency/task_groups/body-gather.py"),
            vec!["A", "B-release", "K-release", "parent-release"],
            false,
        ),
        (
            "body-joined-child",
            include_str!("../fixtures/concurrency/task_groups/body-joined-child.py"),
            vec!["A", "B-release", "K-release", "parent-release"],
            false,
        ),
        (
            "normal",
            include_str!("../fixtures/concurrency/task_groups/normal.py"),
            vec!["B", "A", "discarded", "parent-release"],
            false,
        ),
        (
            "failure",
            include_str!("../fixtures/concurrency/task_groups/failure.py"),
            vec!["A", "child-release", "parent-release"],
            false,
        ),
        (
            "host-cancel",
            include_str!("../fixtures/concurrency/task_groups/host-cancel.py"),
            vec!["$cancel", "A-release", "B-release", "parent-release"],
            true,
        ),
        (
            "root-error",
            include_str!("../fixtures/concurrency/task_groups/root-error.py"),
            vec!["control", "child-release"],
            false,
        ),
        (
            "root-shutdown",
            include_str!("../fixtures/concurrency/task_groups/root-shutdown.py"),
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
            include_str!("../fixtures/concurrency/cleanup_boundaries/root-cyclic-false.py"),
            vec!["control", "child-release"],
        ),
        (
            "run-cyclic-error",
            include_str!("../fixtures/concurrency/cleanup_boundaries/root-cyclic-true.py"),
            vec!["control", "child-release"],
        ),
        (
            "ordinary-finally",
            include_str!("../fixtures/concurrency/cleanup_boundaries/body-direct-finally.py"),
            vec!["A", "child-release", "parent-release"],
        ),
        (
            "ordinary-context",
            include_str!("../fixtures/concurrency/cleanup_boundaries/body-context.py"),
            vec!["A", "child-release", "parent-release"],
        ),
        (
            "ordinary-helper",
            include_str!("../fixtures/concurrency/cleanup_boundaries/body-helper-transfer.py"),
            vec!["A", "child-release", "parent-release"],
        ),
        (
            "ordinary-recovery",
            include_str!("../fixtures/concurrency/cleanup_boundaries/body-recovery.py"),
            vec!["A", "recovery", "helper-release", "child-release", "parent-release"],
        ),
        (
            "transferred-direct",
            include_str!("../fixtures/concurrency/cleanup_boundaries/body-transferred-direct.py"),
            vec!["A", "B", "child-release", "parent-release"],
        ),
        (
            "run-in-task",
            include_str!("../fixtures/concurrency/cleanup_boundaries/run-in-scheduler-task-owned.py"),
            vec!["control", "child-release", "outside"],
        ),
        (
            "run-in-task-nested-group",
            include_str!("../fixtures/concurrency/cleanup_boundaries/run-in-nested-task-group-owned.py"),
            vec!["control", "body", "child-release", "outside"],
        ),
        (
            "host-run",
            include_str!("../fixtures/concurrency/cleanup_boundaries/root-host.py"),
            vec!["control", "$cancel", "child-release"],
        ),
        (
            "host-cleanup",
            include_str!("../fixtures/concurrency/cleanup_boundaries/body-host.py"),
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
            include_str!("../fixtures/concurrency/public_queues/reserved-get-cancel.py"),
            vec!["control", "verified"],
        ),
        (
            "reserved-put-cancel",
            include_str!("../fixtures/concurrency/public_queues/reserved-put-cancel.py"),
            vec!["control", "verified"],
        ),
        (
            "inspection-lazy",
            include_str!("../fixtures/concurrency/public_queues/inspection-lazy.py"),
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
            include_str!("../fixtures/concurrency/public_queues/unbounded.py"),
            vec!["stored"],
        ),
        (
            "bounded-fifo",
            include_str!("../fixtures/concurrency/public_queues/bounded-fifo.py"),
            vec!["full", "empty"],
        ),
        (
            "getters-fifo",
            include_str!("../fixtures/concurrency/public_queues/getters-fifo.py"),
            vec!["empty", "done"],
        ),
        (
            "cancel-get",
            include_str!("../fixtures/concurrency/public_queues/cancel-get.py"),
            vec!["blocked", "done"],
        ),
        (
            "cancel-put",
            include_str!("../fixtures/concurrency/public_queues/cancel-put.py"),
            vec!["blocked", "done"],
        ),
        (
            "delivered-cancel",
            include_str!("../fixtures/concurrency/public_queues/delivered-cancel.py"),
            vec!["control"],
        ),
        (
            "group-failure",
            include_str!("../fixtures/concurrency/public_queues/group-failure.py"),
            vec!["fail", "child-release", "parent-release"],
        ),
        (
            "ordinary-cleanup",
            include_str!("../fixtures/concurrency/public_queues/ordinary-cleanup.py"),
            vec!["fail", "child-release", "parent-release"],
        ),
        (
            "run-shutdown",
            include_str!("../fixtures/concurrency/public_queues/run-shutdown.py"),
            vec!["control", "child-release"],
        ),
        (
            "protected-cleanup",
            include_str!("../fixtures/concurrency/public_queues/protected-cleanup.py"),
            vec!["body", "release", "parent-release"],
        ),
        (
            "host-cleanup",
            include_str!("../fixtures/concurrency/public_queues/host-cleanup.py"),
            vec!["$cancel", "$cancel", "release", "parent-release"],
        ),
        (
            "host-group",
            include_str!("../fixtures/concurrency/public_queues/host-group.py"),
            vec!["$cancel", "$cancel", "child-release", "parent-release"],
        ),
        (
            "misuse-reawait",
            include_str!("../fixtures/concurrency/public_queues/misuse-reawait.py"),
            vec!["checked"],
        ),
        (
            "reservations",
            include_str!("../fixtures/concurrency/public_queues/reservations.py"),
            vec!["put-reserved", "get-reserved"],
        ),
        (
            "cyclic-blocked-put",
            include_str!("../fixtures/concurrency/public_queues/cyclic-blocked-put.py"),
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

/// Authored cancellation owns cyclic arguments just like delivered cancellation.
/// Check live heaps after completion and original/restored disposal at every cut.
#[test]
fn public_authored_cancellation_snapshot_ownership() {
    for source in [
        include_str!("../fixtures/concurrency/authored_cancellation/tasks.py"),
        include_str!("../fixtures/concurrency/authored_cancellation/groups.py"),
        include_str!("../fixtures/concurrency/authored_cancellation/scopes.py"),
    ] {
        let definitions = source.strip_suffix("asyncio.run(main())\n").expect("entry");
        let mut repl = new_repl();
        repl.feed_run(
            definitions,
            vec![("exchange".into(), MontyObject::function("exchange", None))],
            PrintWriter::Disabled,
        )
        .unwrap();
        let baseline = repl.heap_entry_count();
        for _ in 0..3 {
            let mut progress = repl
                .feed_start("asyncio.run(main())", vec![], PrintWriter::Disabled)
                .unwrap();
            let mut cuts = 0;
            while !matches!(progress, ReplProgress::Complete { .. }) {
                cuts += 1;
                assert!(cuts <= 6);
                let bytes = dump("authored.py", None, &[], SessionRef::Suspended(&progress)).unwrap();
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
                                .map(|id| (id, ExtFunctionResult::Return(MontyObject::string("ok"))))
                                .collect(),
                            PrintWriter::Disabled,
                        )
                        .unwrap()
                };
            }
            let (completed, output) = progress.into_complete().unwrap();
            assert_eq!(output, MontyObject::string("\"ok\""));
            repl = completed;
            repl.feed_run("pulse = []\npulse = None", vec![], PrintWriter::Disabled)
                .unwrap();
            assert_eq!(repl.heap_entry_count(), baseline, "completed history: {source}");
        }
    }
}
