# Protected cleanup gather regressions

Additive checks outside the frozen first native-async milestone. Run after a fresh
`native_async/build.py` build:

```sh
python3 tools/coroutines/native_async_cleanup/verify.py --association <build>/native-async-association.json --output <new-directory>
```

Uses the original transports and restore/dispose/equivalence checker. Checks the
independent review's exact cleanup-gather-error program without repeated cancellation,
nested gathered cleanup, both settlement orders, a second child error, a direct
external sibling, failure during gather setup and normal error unwind. Checks require
delayed parent release, protected replies across repeat cancellation and retained
first-error terminal diagnostics, on native and actual Wasm. These do not establish general
concurrent body cancellation or TaskGroup behavior. Normal error unwind and
before-teardown ownership checks live in `generator_runtime.rs`.
