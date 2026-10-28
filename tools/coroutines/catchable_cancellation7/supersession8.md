# Error-first expectation supersession map

Frozen before implementation. Historical suites and their source assertions stay
byte-identical. Rows identify their old expected fields, not a request to edit them.
All parameter ranges below are exhaustive Cartesian products, not samples. Every
backend/mode shares the replacement. D/C/E use gates8.md notation. Historical ENTRY
uses marker `['delivered',reason]`, not the core's `['caught',reason]`.

| Old suite / case identifier | Old expectation | Replacement and reason |
|---|---|---|
| native1/active_dispatch.py `selector-{implicit}-{match}-{outer}`, each bool False/True (8 cases) | terminal error TypeError: ORIGINAL, value null | reason ''; error null; D(TypeError,ORIGINAL); value {'value':'caught','trace':[['cleanup','ok']] + ([['outer','done']] if outer else []) + [['delivered','']]}. Active selector is interrupted before continuation; required outer cleanup still runs. |
| same `expression-continuation`, `selector-finally-return` | TypeError: ORIGINAL | reason ''; error null; D(TypeError,ORIGINAL); value {'value':'caught','trace':[['cleanup','ok'],['delivered','']]}. No purchase or original handler. Selector-local return cannot suppress its caller's unresolved error. |
| same `sync-False`, `sync-raise` | TypeError: ORIGINAL / TypeError: EXIT | reason ''; error null; D(TypeError,ORIGINAL / EXIT); value {'value':'caught','trace':[['cleanup','ok'],'exit',['delivered','']]}. Capture after exit bookkeeping. |
| same `sync-True` | caught, trace cleanup/exit/delivered | unchanged trace/value/error, explicit interrupted_error=null and value_export_error=null. Suppression precedes delivery. |
| same `protected-False`, `protected-True` | protected-original then outer then delivered | unchanged trace/value/error, explicit summary null; original was handled inside protected cleanup. |
| native1/cleanup_exit.py ERROR_CASES `escaping-original-error`, `context-unsuppressed-original`, `matched-original-error-boundary`, `matched-error-reraise-policy` | respectively TypeError: ORIGINAL / ORIGINAL / handled-original / matched-original | reason FIRST, error null; D(TypeError,respective old message); value {'value':'caught','trace':[[phase,'ok'],['delivered','FIRST']]}, phase context-close for context case, cleanup otherwise. Prospective TypeError handlers do not catch new ValueError. |
| same `escaping-cleanup-error-outer-completes` | TypeError: CLEANUP | reason FIRST, error null; D(TypeError,CLEANUP); value {'value':'caught','trace':[['inner','i'],['outer','o'],['delivered','FIRST']]}. Required outer cleanup finishes before injection. |
| native1/cancellation.py `error-before-cleanup-cancel` | reason HOST, error ValueError: BODY | reason HOST, error ValueError: HOST, value null, D(ValueError,BODY), value_export_error=null. No surrounding catcher. Effects remain body/finally. |
| same `cleanup-error`, `error-after-caught-cancel` | final ValueError: CLEANUP / RECOVERY | error unchanged, summary null. These are postdelivery replacements, not obsolete error-first behavior. |

All other cleanup_exit/cancellation cases retain their authored ordinary traces,
first reasons and final errors; add the two explicit fields (null unless an unresolved
error exists at delivery). They are not globally certified by this map.

## dispatch_exit.py: all fourteen old error-first expectations

Old key is `{backend}-{mode}-{name}-{cleanup_error}` with names below and
cleanup_error False/True. All fourteen formerly demanded only effects cleanup/outer,
terminal TypeError: ORIGINAL or CLEANUP, and value null. New diagnostic is always
D(TypeError,'ORIGINAL' if False else 'CLEANUP'), reason '', export error null.
The old source embeds `assert TRACE == [['cleanup','ok']]` in its outer finally.
Exact historical replay must retain that assertion. It is incompatible with several
now-legal postdelivery traces; do not silently remove it or pass its old assertion.

| name | Exact selected-policy replay |
|---|---|
| literal | ValueError matches; appends 'FORBIDDEN-handler', reraises; outer assertion fails. Effects only cleanup; final AssertionError with the exact annotated message below; value null. |
| explicit | choose appends 'FORBIDDEN', returns ValueError; handler appends 'FORBIDDEN-handler', reraises; outer assertion fails. Effects only cleanup; final AssertionError, value null. |
| implicit | choose appends 'FORBIDDEN', returns None; invalid except type raises, then outer assertion replaces that error. Effects only cleanup; final AssertionError, value null. |
| sync-match | choose appends 'FORBIDDEN', returns TypeError, which does not match signal; outer assertion fails. Effects only cleanup; final AssertionError, value null. |
| async-match | Extra purchase effect after cleanup; fresh batch replies 'paid', repeat does not revoke purchase. Trace gains ['purchase','paid']; TypeError does not match; outer assertion fails. Effects cleanup,purchase; final AssertionError, value null. |
| async-unmatched | Same purchase reply; ValueError matches and adds 'FORBIDDEN-handler'; outer assertion fails. Effects cleanup,purchase; final AssertionError, value null. |
| rethrow | TypeError does not match signal; outer assertion passes; outer reply done; ENTRY catches signal. Effects cleanup,outer; error null; value {'value':'caught','trace':[['cleanup','ok'],['outer','done'],['delivered','']]}. |

For async cases preserve original two batch-file bytes as provenance, but name the
new policy batch set separately: cancel('',cleanup reply ok), repeat(REPEAT,
purchase reply paid); no outer reply is consumed because assertion fails before
outer wait. The historical second reply 'done' instead produces ['purchase','done']
and the same AssertionError terminal: retain this exact-batch replay too. No claim
that either purchase reply is authorized. gates8 authority denies real purchases.
The selected-policy semantic tests without these old assertions are the core
prospective-selector-after-delivery and gates8 signal-selector/authority rows.

With the historical default assertion annotations (120 bytes per operand), the
exact final normalized error strings for the fresh 'paid' variant are:

- literal: `AssertionError: assert [['cleanup', 'ok'], 'FORBIDDEN-handler'] == [['cleanup', 'ok']]`
- explicit: `AssertionError: assert [['cleanup', 'ok'], 'FORBIDDEN', 'FORBIDDEN-handler'] == [['cleanup', 'ok']]`
- implicit and sync-match: `AssertionError: assert [['cleanup', 'ok'], 'FORBIDDEN'] == [['cleanup', 'ok']]`
- async-match: `AssertionError: assert [['cleanup', 'ok'], ['purchase', 'paid']] == [['cleanup', 'ok']]`
- async-unmatched: `AssertionError: assert [['cleanup', 'ok'], ['purchase', 'paid'], 'FORBIDDEN-handler'] == [['cleanup', 'ok']]`

The exact-original-batch variant substitutes the literal `'done'` for `'paid'` in
the last two error strings. No operand reaches the annotation cap. Do not disable
annotations or silently discard the message when checking these default-config replays.

## Independent historical replays

Seven origin cases in cases.json map old outputs to their full literal replacement
observations there: review4 handler-expression-side-effect/new-effect/outer-cleanup;
review5 matching-started-before-cancel and matching-started-before-cancel-outer;
review6 unprotected-selector and actual-owning-return. The last permits its literal
FORBIDDEN-entry-return marker because outer return suppresses an already-delivered
signal; diagnostic ORIGINAL remains. Do not infer forbiddenness from marker names.
Review6 nested one/three selectors use gates8 nested-1/nested-3; review6 first-in-sync
exit False/True/raise use core exit-False-False/True/raise (ORIGINAL/null/EXIT).
Review5 explicit/implicit matching/nonmatching use the eight active_dispatch rows;
review5 protected caller controls use the protected rows unchanged.

This map supersedes expectation identifiers, not historical results or reports.
Original guest and batch bytes/hashes are in repair8 evidence originals for the seven
embedded historical cases; all other historical inputs remain in place and are hashed
by before-sha256.json. The implementation receipts must retain exact input bytes for
every replay, including any separately named amended batch set.
