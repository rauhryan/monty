# Additive TaskGroup checks

These literal programs use the unchanged native_async transports and strict checker.
The fixed contract, cases, verifier and adapters are not modified.

```
python3 tools/coroutines/native_async_groups/verify.py --association <build>/native-async-association.json --output <fresh-output>
.venv/bin/python tools/coroutines/native_async_groups/reference.py
```

Every accepted native/actual-Wasm coordinate requires literal boundaries, independent
snapshot disposal, two fresh restores from preserved original bytes at every effect
cut, and uninterrupted equivalence. Normal/reversed settlement, nested same-owner
groups, discarded handles, duplicate gathered joins, failure while body is suspended,
sibling cleanup failure aggregation, protected cleanup, repeated guest/host
cancellation, normal/error root shutdown and the exact review3 body-obligation repro
are separate cases. Rust tests additionally check invalid/foreign admission,
repeated entry, cached identity, cyclic exception arguments and heap census before
teardown on original/restored abandonment and completed execution.

ExceptionGroup and except* are unsupported: failure.py accepts CPython's exceptions
attribute or Monty's documented RuntimeError args[0] tuple. It asserts every error,
not only the first. CPython reference scripts do not define platform cancellation,
protected cleanup or durability semantics. The known historical ownership exclusions
and broad test failures remain outside this additive suite; no new exclusion is used.
