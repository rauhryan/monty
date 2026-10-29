//! Compiler-only checks may run before the protected heap hooks are approved.
//! No test here executes a generator or allocates a Generator heap payload.
use monty::MontyRun;
use monty_types::{CompileOptions, ExcType};

#[test]
fn milestone_sources_compile() {
    for source in [
        include_str!("../../../tools/coroutines/cases/basic.py"),
        include_str!("../../../tools/coroutines/cases/lexical.py"),
        include_str!("../../../tools/coroutines/cases/single_cut.py"),
        include_str!("../../../tools/coroutines/cases/combined.py"),
        include_str!("../../../tools/coroutines/cases/completion.py"),
        include_str!("../../../tools/coroutines/cases/generator.py"),
    ] {
        MontyRun::new(source.to_owned(), "generator.py", vec![], CompileOptions::default()).unwrap();
    }
}

#[test]
fn yield_does_not_escape_its_lexical_function_scope() {
    for source in [
        "yield 1",
        "def outer():\n    class C:\n        x = (yield 1)",
        "def outer():\n    f = lambda: (yield 1)",
        "async def outer():\n    yield 1",
    ] {
        let error = MontyRun::new(source.to_owned(), "generator.py", vec![], CompileOptions::default()).unwrap_err();
        assert_eq!(error.exc_type(), ExcType::SyntaxError);
    }
}

/// Elements, filters, inner iterables and targets belong to the implicit scope.
#[test]
fn comprehension_scopes_reject_yield() {
    for expression in [
        "[YIELD for x in [1]]",
        "{YIELD for x in [1]}",
        "{YIELD: x for x in [1]}",
        "{x: YIELD for x in [1]}",
        "(YIELD for x in [1])",
        "[x for x in [1] if YIELD]",
        "{x for x in [1] if YIELD}",
        "{x: x for x in [1] if YIELD}",
        "(x for x in [1] if YIELD)",
        "[x for x in [1] for y in YIELD]",
        "{x for x in [1] for y in YIELD}",
        "{x: x for x in [1] for y in YIELD}",
        "(x for x in [1] for y in YIELD)",
        "[[y for y in YIELD] for x in [1]]",
        "[x for x in [1] if [y for y in YIELD]]",
        "[x for x in [1] for y in [z for z in YIELD]]",
        "[x for a[YIELD] in [1]]",
    ] {
        for suspension in ["(yield x)", "(yield from [x])"] {
            let source = format!("def f():\n    return {}", expression.replace("YIELD", suspension));
            let error =
                MontyRun::new(source.clone(), "generator.py", vec![], CompileOptions::default()).expect_err(&source);
            assert_eq!(error.exc_type(), ExcType::SyntaxError, "{source}");
        }
    }
}

/// The first iterable retains the surrounding scope, including through nesting.
#[test]
fn comprehension_outer_iterables_accept_yield() {
    for expression in [
        "[x for x in YIELD]",
        "{x for x in YIELD}",
        "{x: x for x in YIELD}",
        "(x for x in YIELD)",
        "[x for x in [y for y in YIELD]]",
        "([x for x in [1]], YIELD)",
    ] {
        for suspension in ["(yield [1])", "(yield from [[1]])"] {
            let source = format!("def f():\n    return {}", expression.replace("YIELD", suspension));
            MontyRun::new(source.clone(), "generator.py", vec![], CompileOptions::default()).expect(&source);
        }
    }
}

/// Comprehensions must not create permission to yield at module scope.
#[test]
fn comprehension_outer_iterables_require_enclosing_function() {
    for source in [
        "[x for x in (yield [1])]",
        "{x for x in (yield [1])}",
        "{x: x for x in (yield [1])}",
        "(x for x in (yield [1]))",
    ] {
        let error = MontyRun::new(source.to_owned(), "generator.py", vec![], CompileOptions::default()).unwrap_err();
        assert_eq!(error.exc_type(), ExcType::SyntaxError, "{source}");
    }
}

/// Actual version-14 bytes have neither function generator flags nor frame
/// activation routes. Loading defaults those fields without invoking guest code.
#[test]
fn previous_dump_defaults_generator_metadata() {
    let loaded = monty::Dump::load(include_bytes!("fixtures/dump_v14.bin")).unwrap();
    assert_eq!(loaded.script_name, "fixture.py");
    let monty::Session::Suspended(progress) = loaded.state else {
        panic!("expected suspended legacy fixture")
    };
    let call = progress.into_function_call().expect("legacy host cut");
    // Contextual physical disposal, not ordinary Rust drop of a snapshot.
    drop(call.into_repl());
}

/// The explicit dialect is scoped to one compilation and rejects unsupported surfaces.
#[test]
fn host_async_dialect_boundaries() {
    let directive = "from __monty_host__ import host_coroutines\n";
    for body in [
        "async def f():\n    yield 1",
        "async def f():\n    yield from g()",
        "async def f():\n    return [await g() for x in [1]]",
        "def f():\n    return await g()",
        "await g()",
        "def f():\n    from __monty_host__ import host_coroutines",
        "pass\nfrom __monty_host__ import host_coroutines",
    ] {
        let source = format!("{directive}{body}");
        assert!(
            MontyRun::new(source.clone(), "host.py", vec![], CompileOptions::default()).is_err(),
            "{source}"
        );
    }
    for source in [
        include_str!("../../../tools/coroutines/async10_guest.py").to_owned(),
        format!(
            "{directive}{}",
            include_str!("../../../tools/coroutines/scope10_bodies.py")
        ),
    ] {
        MontyRun::new(source, "host.py", vec![], CompileOptions::default()).unwrap();
    }
    // A separate ordinary compile still uses native Coroutine/await.
    MontyRun::new(
        "async def f():\n    return 1\nawait f()".to_owned(),
        "native.py",
        vec![],
        CompileOptions::default(),
    )
    .unwrap();
}

/// Async context syntax must not inherit permission across lexical function/class boundaries.
#[test]
fn native_async_context_lexical_boundaries() {
    for source in [
        "async with C():\n    pass",
        "async def outer():\n    def inner():\n        async with C():\n            pass",
        "async def outer():\n    class C:\n        async with D():\n            pass",
    ] {
        assert!(
            MontyRun::new(source.to_owned(), "context.py", vec![], CompileOptions::default()).is_err(),
            "{source}"
        );
    }
    MontyRun::new(
        "async def f():\n    async with C():\n        pass".to_owned(),
        "context.py",
        vec![],
        CompileOptions::default(),
    )
    .unwrap();
}
