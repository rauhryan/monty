# Host capabilities and durable effect identity (bounded scaffold)

The host owns `Journal`; the guest gets only three authored async contracts and
opaque references. `Host` reuses `stepping.Sandbox` for resume/cancellation and
`native1.adapter.invoke` for source start. It does not replace the native Scheduler.
No network, real money, runtime changes, or generic arbitrary-function dispatcher.

Run from the repository with existing authenticated native/Wasm artifacts:

```sh
.venv/bin/python tools/coroutines/capabilities/verify.py \
  --inventory playground/coroutine-implementation/capabilities-repair2/preserved.json \
  --output playground/coroutine-implementation/capabilities-repair2/verification-fresh
```

Use a fresh output directory on reruns. The historical default verifier authenticates binding SHA256
`67de000ec147cf127d725f719a2f41bd620bf1c9f276d1fd651c21af52bd3097`, its
artifacts/shims, runtime source digest and build inputs, checks disk budgets, runs
unit tests and both actual backends, and records hashes of this new scaffold and
the stepping sources separately. It never updates old bindings.
After a runtime rebuild, pass `--binding` and `--binding-sha256` for the new genuine build receipt,
plus a current preservation inventory; historical shared binaries are stale.
Repair 2's preservation inventory excludes exactly the four explicitly changed host
files; its separate final audit records their original/current hashes and diffs.
The old inventory and source copies remain immutable evidence, not current-source receipts.
Run the additional genuine-backend regressions with a fresh output path:

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=tools/coroutines/capabilities \
  .venv/bin/python tools/coroutines/capabilities/tests/repair.py \
  --output playground/coroutine-implementation/capabilities-repair2/regressions-fresh
```

## Protocol and trust boundary

A guest request is data, for example:

```json
{"id":0,"cap":"host-issued-reference","op":"charge","args":{"resource":"account-1","amount":42}}
```

`id` is only a local descriptor label, never a durable identity or cleanup grant.
This bounded driver routes replies to the transport's sole real pending future;
both transports assert that there is exactly one. The registry looks up `cap`, validates the
host-authenticated tenant/run context, exact operation, exact argument keys and
resource, integer type and per-effect limit. A guessed name confers no authority.
Random handles are bearer references, not intrinsically unforgeable strings.
There is no network authentication, token secrecy proof, cryptographic sealing,
or protection against an attacker controlling the host database or Python process.

Trusted host provisioning creates run and lineage IDs and grants capabilities.
Only `reserve`, `charge`, and `release` are supported. A release grant is limited to
one preselected reservation (`hold-1` in this demo). Release is independently
pregranted; guest labels, `finally`, or assertions of cleanup confer no authority.
The demo service allocates the known reservation; dynamic reservation provisioning
and aggregate spending quotas are outside this slice. Limits are per effect.

Host-held checkpoint records contain `{id, lineage, parent, edge, snapshot,
backend, request, effect}`. The guest never supplies `effect`, lineage authority,
or checkpoint bytes to a settlement. `capture` accepts output only from the
trusted compatible producer and is a **privileged embedding method**, as are
`create_run`, `grant`, `fork`, and cancellation/revocation. Do not expose these as
RPCs to guests. `Host.state` loads by checkpoint and validates tenant/run/lineage;
there is no API to pair caller-provided bytes with another run. Custody supplies
provenance; successful Monty deserialization is not authentication.

The first captured successor of `(lineage,parent,edge)` is canonical. A repeated
execution with the same descriptor/backend reuses that stored successor and its
effect identity; divergent descriptors reject. New producer bytes on replay are
not adopted. Thus actual replay runs the saved VM but does not create a second
settlement branch. A transport `effect` transition always creates a new effect,
even when the guest resets its label and repeats the entire descriptor.
Only an explicit transport `waiting` response to a cancellation-only turn retains
the parent effect. `Suspended.retained_wait` carries this trusted transition;
`capture(..., retained_wait=True)` is privileged producer metadata, never guest input.
Descriptor/backend equality checks consistency but does not establish identity;
snapshot equality is not used. Canonical replay also checks retention consistency.
`fork` is an explicit host grant: it copies the current cut
into a new lineage with a new effect identity, only for an active run. Capabilities
remain run-scoped and current revocations still apply. No automatic effectful fork
is authorized by copying a snapshot. Old cuts are valid replay checkpoints, not
fresh authority; mismatched lineages and substituted payloads reject.

## State transitions and crash contract

SQLite transactions (`BEGIN IMMEDIATE`, synchronous FULL) serialize admission:

- Absent effect: validate current authority, durably insert intent, commit, then
  dispatch the one typed fake service operation with the host effect ID as key.
- Pending effect: duplicate submission raises `Indeterminate`; never redispatch.
- Completed effect: return the recorded outcome without invoking the service.
  Identity reuse with different capability, operation, arguments or label rejects.
- After service completion: persist outcome before reporting it to the caller.

Intent admission is the authority linearization point. Cancellation/revocation
that commits before admission blocks new business effects; one that commits after
admission does not retract that admitted call, even if dispatch has not yet begun.
Completed effects never roll back. Pending effects remain pending across restart;
there is no lease expiry or retry timer.

A crash before dispatch and a crash after dispatch but before completion recording
both leave pending intent. `reconcile` only looks up the downstream key and exact
request. A result resolves pending; absence remains indeterminate. It never retries
non-idempotent work. The separate deterministic fake service atomically stores
mutation/key/result in its own SQLite database. This is an explicit downstream
contract, not generic exactly-once I/O or a distributed transaction. Fault tests
raise at committed boundaries and reopen both stores; they do not emulate power
loss, filesystem corruption, or every SQLite/platform failure.

Revocation blocks NEW effects, but completed results remain replayable, including
legitimate release results. Cancelled run state and first reason live outside the
guest snapshot. Every driver resume consults that state and reasserts native
cancellation when applicable. New reserve/charge requests fail after cancellation;
only an unrevoked, independently scoped release grant admits new cleanup work.
Cancellation does not itself revoke release; explicit release revocation does.

## Executable scope and limits

`test_journal.py` supplies independent literal negative descriptors, cross-tenant
and cross-run ownership cases, mismatched lineage/checkpoint, modified payloads,
restart/revocation, three crash windows, explicit fork and deterministic concurrent
submissions over separate database connections. Its synthetic snapshot bytes are
only journal test fixtures, never passed to Monty.

`demo.py` uses genuine current native and Wasm snapshots, retained in host SQLite:
replay of the same payment snapshot charges once; revoking an unsettled payment
handle before restoring denies payment and still releases; cancellation at payment
allows release and denies old payment; first cancellation inside protected cleanup
now finishes cancellation without exposing a subsequent payment request.
The literal VM tests also assert that no post-cleanup trace marker executes before delivery.
Host rejection remains an independent effect-boundary defense, including when guest code catches cancellation and requests more work.
Every runtime advance is a fresh process. Reports record checkpoint digests and
literal outcomes; the existing stepping facade does not retain raw resume stdout.

Single task, single wait, synchronous stepping only. No batching, durable
multi-worker scheduling, task groups, migration acceptance, garbage collection of
the journal, storage quotas, terminal-outcome journal, or production credential
management. Runtime timeout is fail-stop, not resumable preemption. A denied result
is injected as an ordinary ValueError; denials are not recorded external effects.
Missing/invalid labels and scalar/list/null descriptors reach schema validation
without host indexing, then receive that error through the outstanding native wait.
`Reply(None, ...)` explicitly selects this single-wait routing, without inventing
a guest label. The optional label check remains for older stepping callers.
The service has no durable negative business-outcome variant: a service exception
after intent leaves it pending, and absent reconciliation remains indeterminate.
This repair does not migrate historical journals containing incorrectly aliased
effects; retain them as evidence and start fresh runs. Their receipts are not
retroactively rewritten or recertified. Restart/replay of repaired journals is tested.
No real side effects may be substituted without defining their own downstream
idempotency/reconciliation contract and reviewing the trusted host surface.

Cross-repo reasoning, paper grounding and review handoff live in
`notes/monty-capability-effect-contract1.md` in the parent context workspace.
