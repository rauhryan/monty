//! Candidate snapshot scenarios release their owners before VM teardown.

use monty::{Dump, MontyRepl, Session, SessionRef, dump};
use monty_types::{CompileOptions, MontyObject, PrintWriter, ResourceTracker};

use super::support::{check_public_fixture_snapshots, checked_run};

#[test]
fn baseline_exception_and_queue_ownership() {
    for source in [
        include_str!("../fixtures/concurrency/baseline/handled.py"),
        include_str!("../fixtures/concurrency/baseline/unwinding.py"),
        include_str!("../fixtures/concurrency/baseline/internal.py"),
        include_str!("../fixtures/concurrency/baseline/queue.py"),
    ] {
        check_public_fixture_snapshots(source);
    }
}

#[test]
fn baseline_idle_roundtrip_and_ownership() {
    let state = include_str!("../fixtures/concurrency/baseline/idle_state.py");
    let continuation = include_str!("../fixtures/concurrency/baseline/idle_resume.py");
    checked_run(&format!("{state}\n{continuation}\nNone"));
    let mut repl = MontyRepl::new("idle.py", ResourceTracker::default(), CompileOptions::default());
    repl.feed_run(state, vec![], PrintWriter::Disabled).unwrap();
    let bytes = dump("idle.py", None, &[], SessionRef::Idle(&repl)).unwrap();
    for _ in 0..2 {
        let Session::Idle(mut copy) = Dump::load(&bytes).unwrap().state else {
            panic!("idle")
        };
        assert_eq!(
            copy.feed_run(continuation, vec![], PrintWriter::Disabled).unwrap(),
            MontyObject::string("\"ok\"")
        );
        let Session::Idle(abandoned) = Dump::load(&bytes).unwrap().state else {
            panic!("idle")
        };
        drop(abandoned);
    }
}
