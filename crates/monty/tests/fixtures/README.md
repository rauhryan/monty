# Snapshot fixtures

Only compatibility with previously written bytes needs a checked-in binary.
Behavioral snapshot tests create current state in memory, serialize it, and independently restore it.

| File                  | Purpose                                                                                    | Origin                                                                          |
| --------------------- | ------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------- |
| `dump_v37.bin`        | Reject genuine state from before the pre-release compatibility reset.                      | VM API extension work based on `ee3f550d`; `FIXTURE_STATE` in `dump_compat.rs`. |
| `dump_v36.bin`        | Reject the genuine preceding snapshot format, before queue work accounting.                | Dedicated CancelledError change, `1667caad`.                                    |
| `dump_v14.bin`        | Reject a genuine upstream snapshot below the supported minimum.                            | Upstream dataclass change, `9000dc6c`.                                          |
| `anyio_scope_v35.bin` | Reject genuine suspended state retaining the old cancellation signal and an active shield. | AnyIO scope change, `e5077481`.                                                 |

The current format is 41; the minimum supported version is 40.
Experimental snapshots have no cross-version compatibility guarantee.
Historical files deliberately retain their original bytes; never regenerate them with the current serializer.
The suite checks every older version header and representative future versions without a binary per version.
Current behavioral snapshots are generated in memory and independently restored twice.
No new maintained binary is needed for a format reset.
Removed experimental snapshots remain in Git history at `5858ff4139fa0495b96a0ef5927f01fa96b5ae13`.

## SHA-256

```text
3ef060e2943c2a4c07bae6f341e165d93663fcd140bd21be569a75d73a047051  anyio_scope_v35.bin
59697b04dd7ac16102938284667d12a3634aa0c35acb0b1297d6beb76f47fd3c  dump_v14.bin
72af94f9396f082c1bcc85562ca0e50106a68c5594bf9013542ff3ca829b5a08  dump_v36.bin
b4a7d974ae30823d67d47bdcf1c0631c80d684c65b7d05d4d318385b1f02cb39  dump_v37.bin
```

## Frozen v40 baseline

`baseline_v40/manifest.json` records six immutable snapshots produced by clean commit `71cef5732232d74751c0ef7c826c226f2d53dd5d`.
Its manifest includes source hashes, producer tree/toolchain/build provenance, snapshot hashes, state assertions and continuation expectations.
The native-produced bytes were restored twice on both native and actual Wasm, including partial abandonment and sticky cancellation.
The snapshot files total 117701 bytes.
Do not regenerate these files; use the capture command to create a separate candidate for a later baseline.
Freezing this producer does not itself enable another snapshot format or promise compatibility with upstream snapshots.

To verify these exact bytes against a current consumer build:

```sh
python3 tools/concurrency/run.py verify-baseline \
  --association playground/consumer-build/association.json \
  --manifest crates/monty/tests/fixtures/baseline_v40/manifest.json \
  --output playground/baseline-v40-check-1
```

## Baseline capture sources

`concurrency/baseline/` contains ordinary Python source for baseline capture.
The common idle source is shared with `dump_compat.rs`; current round trips still generate bytes in memory.
The concurrency runner's `baseline` action captures six selected states into ignored review output and tests the exact native-produced bytes on native and Wasm.
It does not add maintained binaries or change the supported format range.
See `tools/concurrency/README.md` for capture, replay and committed-producer provenance requirements.
