# Milestone 3 core scope contract

The three scope_targets are imported byte for byte from anyio_feasibility,
including their expected upstream traces. No refusal expectation is imported.
This verifier prospectively supersedes the historical scope refusal assertion
only; the historical overall feasibility verifier will become obsolete, not pass.
All additional literal cases/expectations are frozen before candidate execution.
The pinned upstream reference is CPython 3.14.5 / AnyIO 4.11.0. Explicit platform
cases cover sticky FIRST host cancellation and native cleanup/error policies.
Every FunctionCall/ResolveFutures cut uses the established steps runner: genuine
snapshot, original disposal, two restores, alternate disposal, source-byte
preservation and equality to uninterrupted advancement. Successful terminals and
host cancellation terminals are checked strictly, including all null channels.
Cancellation maps explicitly to ValueError; own-source suppression is by exact
object identity. Full backend parity, deadlines, mutable shields and streams
are out of scope. Group failure retains native automatic cleanup protection;
guest level cancellation in finally requires an explicit lexical shield.
