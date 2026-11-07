//! Experimental coroutine scopes, context managers and structured cleanup ownership.

use monty::{Dump, ReplProgress, Session, SessionRef, dump};
use monty_types::{ExtFunctionResult, MontyObject, PrintWriter};

use super::support::{checked_run, new_repl};

/// Count the actual scope-library task/frame graph before teardown, including a
/// host cut inside an executing child (activation pin) and dormant phase cuts.
#[test]
fn scope9_aggregate_original_and_restored_release_before_teardown() {
    let library = include_str!("../fixtures/concurrency/generator_scopes/library.py");
    let helpers = include_str!("../fixtures/concurrency/generator_scopes/ownership.py");
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
    let library = include_str!("../fixtures/concurrency/generator_scopes/library.py");
    let helpers = include_str!("../fixtures/concurrency/generator_scopes/ownership.py");
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
    let library = include_str!("../fixtures/concurrency/async_contexts/guest.py");
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
    let library = include_str!("../fixtures/concurrency/structured_cleanup/library.py");
    let driver = include_str!("../fixtures/concurrency/structured_cleanup/driver.py");
    let helpers = include_str!("../fixtures/concurrency/structured_cleanup/ownership.py");
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
    let library = include_str!("../fixtures/concurrency/structured_cleanup/library.py");
    let driver = include_str!("../fixtures/concurrency/structured_cleanup/driver.py");
    let helpers = include_str!("../fixtures/concurrency/structured_cleanup/ownership.py");
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
    let library = include_str!("../fixtures/concurrency/structured_cleanup/library.py").replace(
        "inner.defer('inner-arbitrary', 'I')",
        "inner.defer('inner-arbitrary', token)",
    );
    let driver = include_str!("../fixtures/concurrency/structured_cleanup/driver.py");
    let helpers = include_str!("../fixtures/concurrency/structured_cleanup/ownership.py");
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
    let library = include_str!("../fixtures/concurrency/structured_cleanup/library.py").replace(
        "inner.defer('inner-arbitrary', 'I')",
        "inner.defer('inner-arbitrary', token)",
    );
    let driver = include_str!("../fixtures/concurrency/structured_cleanup/driver.py");
    let helpers = include_str!("../fixtures/concurrency/structured_cleanup/ownership.py");
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
    let library = include_str!("../fixtures/concurrency/structured_cleanup/library.py");
    let driver = include_str!("../fixtures/concurrency/structured_cleanup/driver.py");
    let helpers = include_str!("../fixtures/concurrency/structured_cleanup/ownership.py");
    let fixture = include_str!("../fixtures/concurrency/structured_cleanup/retained_ownership.py");
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
    let library = include_str!("../fixtures/concurrency/structured_cleanup/library.py");
    let driver = include_str!("../fixtures/concurrency/structured_cleanup/driver.py");
    let helpers = include_str!("../fixtures/concurrency/structured_cleanup/ownership.py");
    let fixture = include_str!("../fixtures/concurrency/structured_cleanup/retained_ownership.py");
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
        include_str!("../fixtures/concurrency/coordinator_stress/bindings.py"),
        include_str!("../fixtures/concurrency/coordinator_stress/kernel.py"),
        include_str!("../fixtures/concurrency/coordinator_stress/coordinator.py"),
        include_str!("../fixtures/concurrency/coordinator_stress/stress_workload.py"),
        include_str!("../fixtures/concurrency/coordinator_stress/ownership.py"),
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

/// The concurrent-I/O slice uses one host-mode parent and two Rust-owned child
/// generator chains. Verify live ownership before heap teardown, including the
/// partially joined cut and repeated lifecycles on the same REPL.
#[test]
fn concurrent_io_live_ownership() {
    let mut repl = new_repl();
    repl.feed_run(
        include_str!("../fixtures/concurrency/concurrent_io/wrappers.py"),
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
        include_str!("../fixtures/concurrency/concurrent_io/children_drain.py"),
        include_str!("../fixtures/concurrency/concurrent_io/children_errors.py"),
        include_str!("../fixtures/concurrency/concurrent_io/children_cancel.py"),
        include_str!("../fixtures/concurrency/concurrent_io/host_cancel.py"),
        include_str!("../fixtures/concurrency/concurrent_io/deferred_cleanup.py"),
        include_str!("../fixtures/concurrency/concurrent_io/gather_host.py"),
        include_str!("../fixtures/concurrency/concurrent_io/gather_first_error.py"),
    ] {
        let mut repl = new_repl();
        repl.feed_run(
            include_str!("../fixtures/concurrency/concurrent_io/wrappers.py"),
            vec![],
            PrintWriter::Stdout,
        )
        .unwrap();
        repl.feed_run(
            include_str!("../fixtures/concurrency/concurrent_io/scope.py"),
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
