# Snapshot fixtures

Only compatibility with previously written bytes needs a checked-in binary.
Behavioral snapshot tests create current state in memory, serialize it, and independently restore it.

| File                  | Purpose                                                                                                  | Origin                                                                          |
| --------------------- | -------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------- |
| `dump_v37.bin`        | Supported-format compatibility: load, resume, generators, aliases, cycles, classes and suspended frames. | VM API extension work based on `ee3f550d`; `FIXTURE_STATE` in `dump_compat.rs`. |
| `dump_v36.bin`        | Reject the genuine preceding snapshot format, before queue work accounting.                              | Dedicated CancelledError change, `1667caad`.                                    |
| `dump_v14.bin`        | Reject a genuine upstream snapshot below the supported minimum.                                          | Upstream dataclass change, `9000dc6c`.                                          |
| `anyio_scope_v35.bin` | Reject genuine suspended state retaining the old cancellation signal and an active shield.               | AnyIO scope change, `e5077481`.                                                 |

The current format is 38 and the minimum supported version is 37.
The genuine v37 fixture checks additive compatibility; current behavioral snapshots are generated in memory.
Historical files deliberately retain their original bytes; never regenerate them with the current serializer.
The compatibility suite checks every older version header and representative future versions without a binary per version.
Removed experimental snapshots remain in Git history at `5858ff4139fa0495b96a0ef5927f01fa96b5ae13`.

## Updating the current fixture

First run `cargo test -p monty --test dump_compat`.
Investigate a compatibility failure before regenerating: a new fixture must not hide an unintended format break.
After an incompatible format change, select a new `FIXTURE_VERSION` matching `DUMP_VERSION` before generating that fixture.
The test refuses to overwrite an older-version fixture.
Generate the selected fixture with:

```sh
UPDATE_DUMP_FIXTURE=1 cargo test -p monty --test dump_compat fixture_dump_loads_and_resumes
```

Then run the suite again without that variable to verify actual load and resume.
On shared build machines, use the established build guard and cache budget for these commands.
Record the producer commit and hash here when adding or intentionally updating a fixture.

## SHA-256

```text
3ef060e2943c2a4c07bae6f341e165d93663fcd140bd21be569a75d73a047051  anyio_scope_v35.bin
59697b04dd7ac16102938284667d12a3634aa0c35acb0b1297d6beb76f47fd3c  dump_v14.bin
72af94f9396f082c1bcc85562ca0e50106a68c5594bf9013542ff3ca829b5a08  dump_v36.bin
b4a7d974ae30823d67d47bdcf1c0631c80d684c65b7d05d4d318385b1f02cb39  dump_v37.bin
```
