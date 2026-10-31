# Frozen supplemental gates 8

These are preimplementation literal tables, not executed VM results. They supplement,
and do not change, the 27 cases / 108 core coordinates. Each row is mandatory on
native and actual Wasm unless explicitly marked native census or transport-specific.
The implementation harness must materialize these inputs without changing the
expected outcomes. Record any unsupported harness case as NOT RUN, never PASS.

Notation: `D(T,m,t=false)` is exactly
`{"type":T,"message":m,"message_truncated":t,"representation":"metadata-only"}`.
`C(v,d,e=null,x=null)` is exactly the six-key terminal
`{"status":"cancelled","reason":"FIRST","value":v,"error":e,"interrupted_error":d,"value_export_error":x}`.
`E(p,t)` is exactly `{"phase":p,"trace":t}`. Empty reason variants replace FIRST
with `""` everywhere, not with a default. Errors below are final-line normalized;
raw full envelopes/tracebacks must be retained. No optional summary keys.

## Dispatch and ownership inputs

The following small scenarios specify guest operations in order; marker strings
are literal trace entries. `wait(name)` emits E(name,current trace); its successful
reply appends `[name,reply]`, like the core's op. Omit that append on revoked/error
replies. Catch bodies append the stated markers and return the stated value.

| ID | Input and cancellation cut | Exact observations / terminal |
|---|---|---|
| helper-local | Raise TypeError('ORIGINAL'); active `except await choose()`; choose catches ValueError around wait('selector'), appends `['local',str(e)]`, returns TypeError. Selected handler appends `['original',str(e)]`, returns 'recovered'. Cancel selector with FIRST and stale reply 'ignored'. | effects `[E('selector',[])]`; C({'value':'recovered','trace':[['local','FIRST'],['original','ORIGINAL']]},D('TypeError','ORIGINAL')). No selector replay. |
| nested-1 / nested-3 | Enter one / three unresolved selector callers with errors OUTER-1..OUTER-3; innermost raises TypeError('INNER-0') and selects at wait('selector'). Outermost entry catches signal, appends ['caught',str(e)], returns 'caught'. | effects `[E('selector',[])]`; C({'value':'caught','trace':[['caught','FIRST']]},D('TypeError','INNER-0')). |
| current-over-S | While selecting for ORIGINAL, helper raises TypeError('CURRENT') and enters required finally wait('cleanup'); cancel with reply 'ok'; outer entry catches signal. | effects `[E('cleanup',[])]`; C({'value':'caught','trace':[['cleanup','ok'],['caught','FIRST']]},D('TypeError','CURRENT')). |
| handled-outer | Bind TypeError('HANDLED') in an outer handler; inside its body wait('body'); cancel; inner catcher appends ['caught',str(e)], returns 'caught'. No unresolved selector. | effects `[E('body',[])]`; C({'value':'caught','trace':[['caught','FIRST']]},null). |
| name-delete | In handler `except TypeError as bound`, wait('body'); cancel, unwind handler; outer signal catcher probes bound and catches NameError, appends 'deleted', returns 'caught'. | effects `[E('body',[])]`; C({'value':'caught','trace':['deleted']},null). Binding must not retain an exception root. |
| completed-cleanup | Raise ORIGINAL under finally wait('cleanup'), then prospective ValueError catcher returns 'caught'; cancel with reply 'ok'. Catcher appends ['caught',str(e)]. | effects `[E('cleanup',[])]`; C({'value':'caught','trace':[['cleanup','ok'],['caught','FIRST']]},D('TypeError','ORIGINAL')). Cleanup count exactly one. |
| signal-bare-raise | Same as completed-cleanup, but catcher reraises without append. | effects `[E('cleanup',[])]`; C(null,D('TypeError','ORIGINAL'),'ValueError: FIRST'). |
| signal-selector-raises | Same pending ORIGINAL/cleanup, prospective selector appends 'select' then raises TypeError('SELECTOR'); no outer catcher. | effects `[E('cleanup',[])]`; C(null,D('TypeError','ORIGINAL'),'TypeError: SELECTOR'); preterminal trace [['cleanup','ok'],'select']. |
| postdelivery-recovery | core recovery-repeat-keeps-reply plus a legacy ordinary helper call immediately after recovery wait; append its 'legacy-ok' return; later TypeError('LATER') caught normally. | Exactly the core's effects/batches; terminal reason ''; value {'value':'recovered','trace':[['caught-in-body',''],['recovery','ok'],'legacy-ok','recovered-later']}; status cancelled; error, interrupted_error and value_export_error null. No activation refusal/reinjection/filter/revocation. |

For immediate cases execute both live VM-frame and saved Task-frame paths. Deliberately
place a different handled error in the unrelated parked VM stack: diagnostic remains
the task's ORIGINAL/INNER-0, never that unrelated error. Assert saved instruction_ip,
caller ip-1 and exception-stack bases resolve the same logical ownership after restore.

## Request ordering and authority

Use helper-local above, but its local signal catcher then waits at 'recovery' before
returning TypeError. The first request ID is A, the fresh recovery ID is B (actual
opaque IDs captured at registration, never guessed integer literals).

| ID | Host batches | Exact gate |
|---|---|---|
| cancel-error | FIRST + error ValueError('STALE') for A; then reply 'ok' for B | No STALE summary/error; effects selector then recovery with trace [['local','FIRST']]; final trace [['local','FIRST'],['recovery','ok'],['original','ORIGINAL']]; same recovered C and ORIGINAL diagnostic. |
| stale-and-valid | FIRST only; then REPEAT + reply 'ignored' for A + reply 'ok' for B | Same effects/terminal as cancel-error; B consumed once; no new delivery. |
| empty-first | Empty cancel only; repeat FIRST + stale A + valid B | Same trace with local reason empty and terminal reason empty; ORIGINAL diagnostic unchanged. |
| protected-error | While protected wait cleanup for ORIGINAL, FIRST plus error TypeError('EXIT') for its valid ID | Required error replacement completes; signal catcher returns caught; diagnostic D('TypeError','EXIT'), never ORIGINAL. |
| authority | Prospective selector-for-signal reaches purchase after cleanup reply ok and delivery; real capability host denies purchase with ValueError('DENIED'); outer entry catches and returns caught | effects cleanup then purchase; trace [['cleanup','ok'],'selector-for-signal',['caught','DENIED']]; C({'value':'caught','trace':that trace},D('TypeError','ORIGINAL')); purchase calls attempted=1, authorized=0, charge=0, journal commits=0. No second injection even if guest catches DENIED. |

The authority row is a fresh host-ledger case, not the historical transport replay
that supplies 'paid'. Freeze and retain the denial reply bytes before runtime work.
Repeat all first/repeat variants across before-delivery and after-delivery restore.

## Diagnostic boundaries and export

For each metadata row, a protected finally wait leaves a catchable TypeError with
exactly the stated stored SimpleException message at delivery. Signal catcher returns
'caught'. The terminal is C('caught',D('TypeError',expected,truncated)); no formatter
is called. For args rows construct the indicated exception args, whose stored message
is null; retain aliases only as ordinary guest locals and drop them before census.

| ID / stored input | Exact expected message | truncated |
|---|---|---|
| null | null | false |
| empty | '' | false |
| cap4096: 'a' repeated 4096 | 'a' repeated 4096 | false |
| cap4097: 'a' repeated 4097 | 'a' repeated 4096 | true |
| multibyte: 'a' repeated 4095 + 'é' | 'a' repeated 4095 | true |
| multibyte-exact: 'é' repeated 2048 | 'é' repeated 2048 | false |
| args-alias: TypeError([token,token]) | null | false |
| args-cycle: a=[]; a.append(a); TypeError(a) | null | false |
| args-generator: TypeError(generator_object) | null | false |

Copy at most the final prefix; a large stored string must not cause a full temporary
clone. Null is immutable after delivery even if later recovery raises another error.
Validate both exact structured decoders, not JSON-stringified diagnostics.

| Export/fatal ID | Return/input after catching signal | Exact outcome |
|---|---|---|
| root-unsupported | TypeError([1]) | C(null,null,null,'unsupported-value') (existing core) |
| partial-unsupported | ['prefix', shared_supported_object, TypeError([1])] | C(null,null,null,'unsupported-value'); exporter must demonstrably visit prefix before rejection; partial pins and return owner released. |
| partial-depth | ['prefix', nested_list] with nested_list one beyond the actual exporter depth bound | C(null,null,null,'export-depth-limit'); instrument/retain depth bound and partial visited count >0; all partial pins released. |
| export-terminal | Inject actual ExportError::Terminal at export seam after partial graph | fatal/no cancelled Complete; return and partial graph guards run where unwinding is possible. |
| internal | RunError::Internal at delivery/projection seam | fatal/no cancelled Complete, no catchable FIRST, no summary fallback. |
| uncatchable | RunError::UncatchableExc at delivery/projection seam | fatal/no cancelled Complete, no catchable FIRST. |
| allocator/transport | Armed worker allocator limit / envelope transport failure | fatal event or worker termination, never fabricated successful envelope; host reason still FIRST. No claim that hard termination runs cleanup. |

A summary adds zero guest heap roots. Native live REPL census after ordinary roots
are dropped equals the fixture's recorded pre-case baseline (3 -> 3 where the same
fixture applies), before teardown. Cover caught-and-suspended, terminal success,
uncaught signal, later cleanup error, both partial exports, and physical disposal.
No private Wasm census claim; actual Wasm semantics and zero disposal effects required.

## Compatibility literal gates

Wasm admission expects required function `worker.cancellationContract()` -> numeric
u32 7 (policy revision, independent of Proto8/Dump23). Before **any dispatch**, a
missing/non-function export, thrown call, boolean, string, 0, 6 or 8 is refused with
host error code `MONTY_CANCELLATION_CONTRACT_UNSUPPORTED`; dispatch count=0, guest
trace=[], feed/load/cancel count=0. Genuine max7 component lacks the export and must
be rejected. A stub that lacks the export only tests the client branch, not genuine
old-component compatibility. Valid 7 admits once per instance; replacement instance
requires a fresh admission. Ordinary old JS client/new component still dispatches
its unchanged configure and ordinary feed. That control promises no new policy.

Protobuf new client sends Configure.protocol_version=8 without downgrade: genuine
max7 worker rejects configure; no feed/load/cancel sent. Ordinary old client version7
at new Proto8/min5 worker succeeds. Direct native1 library linking is not this test.

Genuine Dump19/20/21/22 with no latch: ordinary continuation succeeds. New cancel
refuses before latch/revocation/results mutation with host RuntimeError diagnostic
`native cancellation requires catchable cancellation policy 7`; no cancelled Complete.
Old pending **or delivered** latch: load/resume admission refuses with the same
compatibility diagnostic before guest execution, even if diagnostic is absent/null.
Current revision7 delivered latch with summary null succeeds and never recaptures.

For a current pending latch, entering any legacy ordinary function, frozen function,
new generator, suspended generator, or a delegated chain with one legacy segment:
reject before installing any frame or mutating generator state. Surface
RunError::UncatchableExc(RuntimeError with the same compatibility text); propagate as
an error/fatal outcome, never a successful cancelled Complete or a catchable FIRST.
No selected guest body marker, partial frame installation, leaked recursion reservation
or mutation to an unentered generator is allowed. This is a compatibility failure,
not successful policy delivery. With no pending latch, or with a **current** delivered
latch, those ordinary activations retain ordinary legacy behavior. No reinjection or
extra filtering is permitted during postdelivery recovery.

## All-cut manifest (separate from the 108-coordinate checker)

Before execution enumerate each case/backend and every external boundary ordinal,
including repeated protected cancellation-only waits, inner/outer cleanup, active
selector, prospective selector-for-signal, and caught recovery. An implementation
acceptance report must contain a row for every enumerated cut; zero omitted cuts.
Each row records source/batches raw-byte SHA256, build/source/artifact association,
actual request IDs, snapshot path/SHA256/header, original process ID/argv/receipts,
original physical dispose receipt, and two distinct restore process IDs/argv/receipts
using identical unmodified snapshot bytes. Both branches must have their own disposal
receipt and exact suffix/terminal comparison with the uninterrupted literal case.
Snapshot original disposal precedes independent restores; include an additional
branch disposed immediately at each cut (no continuation), with zero guest effects.
Duplicate snapshot hashes are allowed for identical waits; cut ordinals are distinct.
Native live census rows identify baseline and pre-teardown counts, not process-exit
counts. Missing process, disposal, hash, cut, or real-Wasm evidence blocks acceptance
regardless of a passing core checker. This is a manifest requirement, not a new
snapshot validator or assertion that all cuts have now been exercised.
