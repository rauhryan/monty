//! Current snapshot round trips and explicit rejection of historical formats.

use insta::assert_snapshot;
use monty::{
    DUMP_VERSION, Dump, DumpError, MIN_SUPPORTED_DUMP_VERSION, MontyRepl, ReplProgress, Session, SessionRef, dump,
};
use monty_types::{CompileOptions, ExecutionClock, MontyObject, NameLookupResult, PrintWriter, ResourceTracker};
use serde::{Deserialize, Serialize};

/// Builds the state the fixture holds: the common heap types, live iterators,
/// functions, classes and a cycle, ahead of a suspension on a host call whose
/// arguments carry heap values.
const FIXTURE_STATE: &str = include_str!("fixtures/concurrency/baseline/idle_state.py");

/// The host call the fixture is suspended on, from inside an `exec()` frame with a
/// globals dict, a `finally` block, an `except` handler and an `__init__`.
const FIXTURE_CALL: &str = "Pending(xs, d, b).result + 1";

/// Fed after restoring and resuming the fixture: every value must have survived.
const FIXTURE_CHECK: &str = r"
assert big == 2**100 and neg == -7 and f == 3.5 and s == 'héllo'
assert b == b'\x00\x01bytes'
assert xs == [1, 2.5, 'three', None, True] and t == (1, 'two')
assert d == {'a': 1, 'b': [2, 3], 3: 'int key'}
assert st == {1, 2, 3} and fs == frozenset({'x', 'y'})
assert list(dq) == [1, 2, 3] and dq.maxlen == 5
assert p == Point(1, 2) and p.x == 1
assert item == Item('widget', 3)
assert c.value == 11 and c.bump() == 12 and Counter.total == 0
assert add5(1) == 6 and part() == 6
assert next(it) == 2
assert str(err) == 'boom' and isinstance(err, ValueError)
assert dt.microsecond == 678901 and dlt.days == 1
assert m.groups() == ('12', 'ab') and pat.sub('X', '1-a 2-b') == 'X X'
assert cycle[0] is cycle
assert next(new_generator) is cycle
assert new_generator.send(cycle) is cycle
assert dormant_generator.send(cycle) is cycle
assert next(new_generator, 'done') == 'done'
assert next(dormant_generator, 'done') == 'done'
assert day == datetime.date(2024, 5, 6) and tm == datetime.time(1, 2, 3)
assert tz.utcoffset(None) == datetime.timedelta(hours=2) and tz.tzname(None) == 'X'
assert raw == b'\x07\x08\x09' and list(rg) == [2, 5, 8]
assert (sl.start, sl.stop, sl.step) == (1, 5, 2)
assert bound() == 13 and c.value == 13
assert list(kv) == ['a', 'b', 3] and list(vv) == [1, [2, 3], 'int key'] and ('a', 1) in iv
fresh = random.Random(42)
fresh.random()
assert rnd.random() == fresh.random()
assert list(dq_it) == [2, 3] and list(t_it) == ['two'] and list(b_it) == [8, 9]
assert list(di_it) == [('b', [2, 3]), (3, 'int key')] and len(list(s_it)) == 2
assert next(call_it) == 7
assert pending_log == ['finally']
assert joined == 'x-y' and list(str_it) == ['b', 'c'] and list(range_it) == [1, 2]
assert list(dk_it) == ['b', 3] and list(dv_it) == [[2, 3], 'int key'] and next(counter) == 6
assert alias == list[int] and opt == (int | None) and str(where) == '/data/file.txt'
assert with_default(1) == 2
assert functools.reduce(lambda a, b: a + b, [1, 2, 3]) == 6
assert add5(2) == 7
(result, c.value)
";

/// Current snapshots preserve the common heap types and suspended frames.
#[test]
fn current_dump_loads_and_resumes() {
    let bytes = current_fixture_dump();
    for _ in 0..2 {
        check_fixture_dump(&bytes);
    }
}

/// Header-only protocol probes: unsupported versions must fail before CBOR decoding.
#[test]
fn unsupported_versions_are_rejected_before_payload() {
    assert_eq!(MIN_SUPPORTED_DUMP_VERSION, DUMP_VERSION);
    for version in 0..MIN_SUPPORTED_DUMP_VERSION {
        let mut header = b"MONTY\0".to_vec();
        header.extend_from_slice(&version.to_le_bytes());
        assert_eq!(
            Dump::load(&header).unwrap_err(),
            DumpError::VersionTooOld {
                found: version,
                min_supported: MIN_SUPPORTED_DUMP_VERSION,
            }
        );
    }
    for version in [DUMP_VERSION + 1, u16::MAX] {
        let mut header = b"MONTY\0".to_vec();
        header.extend_from_slice(&version.to_le_bytes());
        assert_eq!(
            Dump::load(&header).unwrap_err(),
            DumpError::VersionTooNew {
                found: version,
                max_supported: DUMP_VERSION,
            }
        );
    }
}

/// Identity-only state and retained named helpers survive independent restores.
#[test]
fn anyio_group_identity_roundtrips() {
    for named in [false, true] {
        let binding = if named {
            "method = group.start_soon"
        } else {
            "create_task_group = None"
        };
        let invocation = if named {
            "method(child, 42)"
        } else {
            "group.start_soon(child, 42)"
        };
        let preflight = if named {
            "assert method(child, 0) is None"
        } else {
            "pass"
        };
        let expected = if named { "[0, 42]" } else { "[42]" };
        let source = format!(
            r"
import asyncio
from anyio import create_task_group
group = create_task_group()
{binding}
results = []
async def child(value):
    results.append(value)
async def main():
    async with group:
        {preflight}
        assert host_call() == 41
        try:
            group.create_task
            assert False
        except AttributeError:
            pass
        assert {invocation} is None
    assert results == {expected}
    return 42
asyncio.run(main())
"
        );
        let repl = MontyRepl::new("anyio.py", ResourceTracker::default(), CompileOptions::default());
        let mut progress = repl.feed_start(&source, vec![], PrintWriter::Stdout).unwrap();
        while let ReplProgress::NameLookup(lookup) = progress {
            let host = MontyObject::function(lookup.name.clone(), None);
            progress = lookup
                .resume(NameLookupResult::Value(host), PrintWriter::Stdout)
                .unwrap();
        }
        assert!(matches!(&progress, ReplProgress::FunctionCall(call) if call.function_name == "host_call"));
        let bytes = dump("anyio.py", None, &[], SessionRef::Suspended(&progress)).unwrap();
        drop(progress.into_function_call().unwrap().into_repl());
        let frozen_name = b"anyio.TaskGroup.start_soon";
        assert_eq!(bytes.windows(frozen_name.len()).any(|part| part == frozen_name), named);
        for _ in 0..2 {
            let Session::Suspended(progress) = Dump::load(&bytes).unwrap().state else {
                panic!("expected host_call suspension");
            };
            let call = progress.into_function_call().unwrap();
            assert_eq!(call.function_name, "host_call");
            let progress = call.resume(MontyObject::int(41), PrintWriter::Stdout).unwrap();
            let (_, value) = progress.into_complete().unwrap();
            assert_eq!(value, MontyObject::int(42));
        }
    }
}

fn current_fixture_dump() -> Vec<u8> {
    let mut repl = MontyRepl::new("fixture.py", ResourceTracker::default(), CompileOptions::default());
    repl.feed_run(FIXTURE_STATE, vec![], PrintWriter::Stdout).unwrap();
    let mut progress = repl.feed_start(FIXTURE_CALL, vec![], PrintWriter::Stdout).unwrap();
    // `host_call` is looked up from inside `exec_host`, where it is not yet defined
    while let ReplProgress::NameLookup(lookup) = progress {
        let host = MontyObject::function(lookup.name.clone(), None);
        progress = lookup
            .resume(NameLookupResult::Value(host), PrintWriter::Stdout)
            .unwrap();
    }
    let ReplProgress::FunctionCall(call) = &progress else {
        panic!("the fixture must suspend on host_call, got {progress:?}");
    };
    assert_eq!(call.function_name, "host_call");
    let bytes = dump("fixture.py", None, &[], SessionRef::Suspended(&progress)).unwrap();
    // Snapshots own manual heap references; abandon them contextually.
    drop(progress.into_function_call().unwrap().into_repl());
    bytes
}

fn check_fixture_dump(bytes: &[u8]) {
    let loaded = Dump::load(bytes).expect("the current snapshot loads");
    assert_eq!(loaded.script_name, "fixture.py");
    assert!(loaded.type_check.is_none());
    let Session::Suspended(progress) = loaded.state else {
        panic!("fixture was dumped suspended on a host call");
    };
    assert_eq!(progress.tracker().execution_clock(), ExecutionClock::Wall);
    let call = progress.into_function_call().expect("suspended on host_call");
    assert_eq!(call.function_name, "host_call");
    // the call arguments are a value graph of their own inside the dump
    assert_eq!(
        call.args.args().collect::<Vec<_>>(),
        vec![
            MontyObject::list([
                MontyObject::int(1),
                MontyObject::float(2.5),
                MontyObject::string("three"),
                MontyObject::none(),
                MontyObject::bool(true),
            ]),
            MontyObject::dict([
                (MontyObject::string("a"), MontyObject::int(1)),
                (
                    MontyObject::string("b"),
                    MontyObject::list([MontyObject::int(2), MontyObject::int(3)]),
                ),
                (MontyObject::int(3), MontyObject::string("int key")),
            ]),
            MontyObject::bytes(b"\x00\x01bytes".as_slice()),
        ]
    );
    let progress = call.resume(MontyObject::int(41), PrintWriter::Stdout).unwrap();
    let (mut repl, value) = progress.into_complete().expect("host_call resumed to completion");
    assert_eq!(value, MontyObject::int(42));
    repl.feed_run("result = 42", vec![], PrintWriter::Stdout).unwrap();
    repl.feed_run(
        "assert owned_completion.value is cycle and cycle[1] is owned_completion\nassert owned_completion.args is owned_completion.args\nassert delegated_generator.send(cycle) is cycle\ntry:\n    next(delegated_generator)\n    assert False\nexcept StopIteration as done:\n    assert done.value is cycle and done.args[0] is cycle",
        vec![], PrintWriter::Stdout,
    ).unwrap();
    let checked = repl.feed_run(FIXTURE_CHECK, vec![], PrintWriter::Stdout).unwrap();
    assert_eq!(
        checked,
        MontyObject::tuple(vec![MontyObject::int(42), MontyObject::int(13)])
    );
}

/// A current-version dump runs no upgrade, so a dataclass whose generated
/// methods were all rebound keeps its own hooks across a reload.
#[test]
fn rebound_dataclass_methods_survive_a_reload() {
    let mut repl = MontyRepl::new("rebound.py", ResourceTracker::default(), CompileOptions::default());
    repl.feed_run(
        r"from dataclasses import dataclass


@dataclass(frozen=True)
class F:
    x: int


def setter(self, name, value):
    pass


def deleter(self, name):
    pass


F.__init__ = lambda self, x: None
F.__repr__ = lambda self: 'own'
F.__eq__ = lambda self, other: True
F.__hash__ = lambda self: 1
F.__setattr__ = setter
F.__delattr__ = deleter
assert F.__setattr__ is setter
assert F.__delattr__ is deleter
",
        vec![],
        PrintWriter::Stdout,
    )
    .unwrap();
    let bytes = dump("rebound.py", None, &[], SessionRef::Idle(&repl)).unwrap();
    let Session::Idle(mut repl) = Dump::load(&bytes).unwrap().state else {
        panic!("dumped idle");
    };
    repl.feed_run(
        "assert F.__setattr__ is setter
assert F.__delattr__ is deleter",
        vec![],
        PrintWriter::Stdout,
    )
    .unwrap();
}

/// Dict and set entries persist their hash, so the hash of every kind of key
/// without a heap identity is part of the dump contract: a change here needs a
/// `DUMP_VERSION` bump.
#[test]
fn persisted_key_hashes_are_stable() {
    let mut repl = MontyRepl::new("hashes.py", ResourceTracker::default(), CompileOptions::default());
    let hashes = repl
        .feed_run(
            "import json, typing\n(hash(None), hash(...), hash(NotImplemented), hash(int), hash(ValueError), \
             hash(len), hash(json.dumps), hash(typing.Any))",
            vec![],
            PrintWriter::Stdout,
        )
        .unwrap();
    assert_snapshot!(hashes.py_repr(), @"(-7376904247260835724, -8418895208483869890, -7578619387304540654, -6751866591645121981, -8097107109803033201, 2456456309100923238, -5014982954702616963, 2858064283329581446)");
}

/// A record as an older build wrote it.
#[derive(Serialize)]
struct RecordV1 {
    id: u32,
    name: String,
    kind: KindV1,
    retired: bool,
}

#[derive(Serialize)]
enum KindV1 {
    Alpha,
    Beta(u8),
}

/// The same record after every kind of layout edit: a field inserted before
/// the others, one renamed, one removed, one appended, and a variant inserted
/// mid-enum.
#[derive(Debug, PartialEq, Deserialize)]
struct RecordV2 {
    #[serde(default)]
    inserted: Option<String>,
    id: u32,
    #[serde(alias = "name")]
    label: String,
    kind: KindV2,
    #[serde(default)]
    appended: u64,
}

#[derive(Debug, PartialEq, Deserialize)]
enum KindV2 {
    Alpha,
    Gamma,
    Beta(u8),
}

/// Named-field codec behavior, independent of the supported dump version range.
#[test]
fn layout_edits_load_older_encodings() {
    for (old, expected) in [
        (
            RecordV1 {
                id: 1,
                name: "one".to_owned(),
                kind: KindV1::Alpha,
                retired: true,
            },
            RecordV2 {
                inserted: None,
                id: 1,
                label: "one".to_owned(),
                kind: KindV2::Alpha,
                appended: 0,
            },
        ),
        (
            RecordV1 {
                id: 2,
                name: "two".to_owned(),
                kind: KindV1::Beta(7),
                retired: false,
            },
            RecordV2 {
                inserted: None,
                id: 2,
                label: "two".to_owned(),
                kind: KindV2::Beta(7),
                appended: 0,
            },
        ),
    ] {
        let bytes = minicbor_serde::to_vec(&old).unwrap();
        assert_eq!(minicbor_serde::from_slice::<RecordV2>(&bytes).unwrap(), expected);
    }
}

/// Current state preserves a cancelled ancestor, active shield and exact signal.
#[test]
fn anyio_scope_snapshot_resumes_twice() {
    let repl = MontyRepl::new("scope.py", ResourceTracker::default(), CompileOptions::default());
    let mut progress = repl
        .feed_start(
            r"
import asyncio
import anyio
async def main():
    with anyio.CancelScope() as scope:
        scope.cancel()
        try:
            await rejected()
        except asyncio.CancelledError as caught:
            signal = caught
        with anyio.CancelScope(shield=True) as shield:
            assert host_call() == 41
            assert scope.cancel_called and not scope.cancelled_caught
            assert isinstance(signal, anyio.get_cancelled_exc_class())
        await rejected()
    assert scope.cancelled_caught
    return 42
asyncio.run(main())
",
            vec![],
            PrintWriter::Disabled,
        )
        .unwrap();
    while let ReplProgress::NameLookup(lookup) = progress {
        let host = MontyObject::function(lookup.name.clone(), None);
        progress = lookup
            .resume(NameLookupResult::Value(host), PrintWriter::Disabled)
            .unwrap();
    }
    assert!(matches!(&progress, ReplProgress::FunctionCall(call) if call.function_name == "host_call"));
    let bytes = dump("scope.py", None, &[], SessionRef::Suspended(&progress)).unwrap();
    drop(progress.into_function_call().unwrap().into_repl());
    for _ in 0..2 {
        let Session::Suspended(progress) = Dump::load(&bytes).unwrap().state else {
            panic!("suspended");
        };
        let progress = progress
            .into_function_call()
            .unwrap()
            .resume(MontyObject::int(41), PrintWriter::Disabled)
            .unwrap();
        let (_, value) = progress.into_complete().unwrap();
        assert_eq!(value, MontyObject::int(42));
    }
}

/// These genuine producers represent upstream state and the incompatible
/// cancellation signal before queue accounting. Never regenerate.
#[test]
fn historical_snapshots_are_rejected() {
    assert_eq!(MIN_SUPPORTED_DUMP_VERSION, DUMP_VERSION);
    for (bytes, version) in [
        (include_bytes!("fixtures/dump_v14.bin").as_slice(), 14),
        (include_bytes!("fixtures/anyio_scope_v35.bin").as_slice(), 35),
        (include_bytes!("fixtures/dump_v36.bin").as_slice(), 36),
        (include_bytes!("fixtures/dump_v37.bin").as_slice(), 37),
    ] {
        assert_eq!(&bytes[..6], b"MONTY\0");
        assert_eq!(u16::from_le_bytes([bytes[6], bytes[7]]), version);
        assert_eq!(
            Dump::load(bytes).unwrap_err(),
            DumpError::VersionTooOld {
                found: version,
                min_supported: MIN_SUPPORTED_DUMP_VERSION,
            }
        );
    }
}
