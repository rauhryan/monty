//! Native cancellation, protected cleanup and terminal diagnostic ownership.

#[cfg(feature = "cancellation-test-hooks")]
use monty::cancellation_test_hooks;
use monty::{Dump, ReplProgress, Session, SessionRef, dump};
use monty_types::{ExcType, ExtFunctionResult, MontyException, MontyObject, PrintWriter};

use super::support::{checked_run, new_repl};

/// Native1 keeps host-dialect generators on existing Scheduler task frames.
/// Both original and loaded graphs reclaim aliases/cycles before heap teardown.
#[test]
fn native1_scheduler_cleanup_ownership() {
    let definitions = include_str!("../fixtures/concurrency/native_cleanup/semantics.py");
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

/// Cancellation ownership under the supported native policy.
#[test]
fn cancelled_cleanup_live_ownership() {
    let mut repl = new_repl();
    repl.feed_run(
        include_str!("../fixtures/concurrency/native_cleanup/cancellation_ownership.py"),
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

/// Cancellation ownership under the supported native policy.
#[test]
fn cleanup_exit_pending_owner_census() {
    for mode in ["return", "error", "cleanup_error"] {
        let mut repl = new_repl();
        repl.feed_run(
            include_str!("../fixtures/concurrency/native_cleanup/cleanup_exit_ownership.py"),
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

/// Cancellation ownership under the supported native policy.
#[test]
fn cancellation_dispatch_owner_census() {
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

/// Cancellation ownership under the supported native policy.
#[test]
fn cancellation_active_selector_owner_census() {
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
            include_str!("../fixtures/concurrency/cancellation_export/ownership.py"),
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
                cancellation_test_hooks::configure(if mode == "later" { "" } else { mode });
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
            let stats = cancellation_test_hooks::export_stats();
            assert!(stats.visits > 1 && stats.peak_pins > 0 && stats.dropped, "{stats:?}");
            assert_eq!(stats.peak_pins, stats.retired_pins);
        }
        #[cfg(feature = "cancellation-test-hooks")]
        cancellation_test_hooks::configure("");
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
