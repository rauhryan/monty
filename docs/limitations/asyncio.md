# `asyncio` module and `async` / `await`

`async def` functions can suspend on `await`, and the host drives long-running
external calls. The sandbox schedules its own tasks; host event loops execute external coroutines.

## Module surface

The `asyncio` module exposes exactly three functions:

- `asyncio.run(coro)` — runs a coroutine to completion. Returns the value
    the coroutine `return`s, or re-raises an exception from it.
- `asyncio.gather(*awaitables)` — runs awaitables concurrently and returns
    a list of results. Always behaves as `return_exceptions=False`.
    Any keyword argument is rejected with
    `NotImplementedError: gather() does not yet support keyword arguments`,
    where CPython raises
    `TypeError: gather() got an unexpected keyword argument 'X'` because
    `return_exceptions` is a real kwarg there.
- `asyncio.sleep(delay, result=None)` — waits as the session's `sleep` setting
    says, then produces `result`. See
    [below](#asynciosleep-waits-at-the-call-not-at-the-await).

Not implemented (raise `AttributeError`):

`create_task`, `wait`, `wait_for`, `shield`, `to_thread`,
`new_event_loop`, `get_event_loop`, `get_running_loop`, `Queue`, `Lock`,
`Semaphore`, `Event`, `Future`, `Task`, `TaskGroup`, `timeout`,
`timeout_at`, `Timeout`, `as_completed`, `iscoroutine`, `ensure_future`,
the whole `asyncio.subprocess` / `asyncio.streams` / `asyncio.protocols`
surface.

`asyncio.timeout()` / `asyncio.timeout_at()` would be unreachable in any
case: they are async context managers, and `async with` is rejected at parse
time (see [language.md](language.md)).

## `async def` / `await`

- `async def` functions and `await` work; coroutines can call each other.
- **Coroutines are single-shot.** Awaiting the same coroutine object twice
    raises `RuntimeError`. Store the *result*, not the coroutine, if you need
    it again.
- `await` on a non-awaitable raises `TypeError`.
- `async for` and `async with` are **rejected at parse time** (see
    [language.md](language.md)). Async iteration and async context-manager
    protocols do not exist.
- Async comprehensions (`[x async for x in ...]`) are rejected at parse
    time.
- There is no `__await__` protocol. Awaitables are only the things Monty
    knows internally: coroutines from `async def`, gather futures, and external
    function call futures returned by host bindings.

## `asyncio.sleep()` waits at the call, not at the `await`

CPython's `asyncio.sleep()` returns a coroutine that does nothing until it is
awaited. Monty starts waiting at the call; `await` produces `result` when the wait finishes.
The session's `sleep` setting determines concurrency (see [time.md](time.md)):

- `'system'`: all pools answer with futures, so gathered sleeps overlap and sibling tasks can run meanwhile.
    Standard Rust execution and the CLI wait inline, making gathered sleeps sequential.
    An immediately awaited sleep may be answered eagerly if no other task can run.
- `'call_host'`: async handlers in `AsyncMonty` and JavaScript allow gathered sleeps to overlap.
    `OSAccess` is async by default under `AsyncMonty`.
    Sync handlers, including `Monty` callbacks, wait sequentially; results are unchanged.
- `'zero'`: the awaitable settles immediately without yielding to sibling tasks, unlike CPython's `sleep(0)`.
    Zero-delay system sleeps also settle without a round trip.
    Under `'call_host'`, a handler returning a pending future allows sibling tasks to run even for zero delay.

Suspending sleeps count against `max_suspensions`; system sleeps also count against `max_total_sleep`.
Waiting consumes neither execution-time limit (see [time.md](time.md)).

What follows from waiting at the call:

- `asyncio.sleep(...)` whose result is never awaited has still started the
    wait, where CPython runs nothing and warns that the coroutine was never
    awaited.
- A bad `delay` raises at the call rather than at the `await`. The error is the
    one CPython's `delay <= 0` produces —
    `TypeError: '<=' not supported between instances of 'str' and 'int'` — but it
    surfaces one step earlier.
- The value is a future rather than a coroutine, though `type(...).__name__`
    is `coroutine` either way. Its `repr()` is `<coroutine external_future(N)>`,
    not CPython's `<coroutine object sleep at 0x...>`, and awaiting it a second
    time replays the same result where CPython raises
    `RuntimeError: cannot reuse already awaited coroutine`.

`delay` accepts only real numbers, matching CPython's `delay <= 0`: an
`__index__`-able class is rejected here although `time.sleep()` accepts it.
A negative delay waits zero seconds instead of raising, as CPython
effectively does, and one past ~9223372036.85 seconds is clamped to that
maximum rather than raising the `OverflowError` `time.sleep()` raises (see
[time.md](time.md)).
A NaN delay raises CPython's `ValueError: Invalid delay: NaN (not a number)`,
but at the call rather than at the `await`.

## Concurrency model

Concurrency is cooperative and host-driven. `gather` suspends Monty whenever
every branch is blocked on an external call, hands the pending calls to the
host, and resumes when the host returns results. There is no preemption, no
threads and no exposed event loop.

### Siblings left running by a failed `gather` only advance while something else suspends

When one child of a `gather` raises, the siblings keep running as they do in CPython.
They resume only when a host result arrives or when another task awaits: Monty's scheduler runs ready tasks only at
those suspension points and has no idle loop that would otherwise turn.
Code that catches the error and then returns without awaiting again leaves them parked where they were:

```python test="skip"
import asyncio


async def tick(): ...  # awaits a host call


async def raises():
    raise ValueError('x')


async def sibling():
    await asyncio.gather(tick())
    print('sibling finished')


async def main():
    try:
        await asyncio.gather(sibling(), raises())
    except ValueError:
        pass


asyncio.run(main())
```

CPython prints `sibling finished` here, Monty prints nothing.

An external call a sibling already passed to the host is still resolved by the host.
Its result reaches the sibling; a result arriving for a `gather` that has already failed is discarded.

Which task runs next also differs.
A task resumed by a host result runs to its next suspension before any other ready task gets a turn, where CPython's
loop takes them in the order they were woken.
Among the tasks that do resume, only the ordering differs.
This applies to all async code, not only to siblings of a failed `gather`.

### `gather` does not start its children until it is awaited

CPython's `gather` schedules each awaitable as a task straight away, so the children run whether or not the result is
ever awaited.
Monty holds the awaitables and spawns nothing until the `await`, so a `gather(...)` whose result is discarded runs no
code at all:

```python test="skip"
import asyncio


async def boom():
    print('boom ran')
    raise ValueError('x')


asyncio.gather(boom())  # CPython prints `boom ran`, Monty runs nothing
```

Awaiting the gather later still runs the children, and a gather that has already failed re-raises its cached
exception on every later await.

Neither does Monty report the exception CPython loses track of here.
CPython prints `_GatheringFuture exception was never retrieved`, a `future:` line and the traceback to stderr when a
gather holding an unretrieved exception is collected.
In Monty that gather never ran, and no sandboxed code can write to stderr in any case — `print()` rejects its `file`
argument.

### Nesting `gather` inside `gather` is bounded by `max_memory`

CPython nests gathers as deeply as memory allows, and the depth costs nothing beyond the futures themselves.
Monty holds a walk frame per level while it commits the nest, so awaiting a deep nest costs memory on top of what the
nest already occupies.
A session with `max_memory` set can therefore build a nest it cannot await: the `await` ends the run with
`MemoryError: memory limit exceeded`, which sandboxed code cannot catch (see [resource_limits.md](resource_limits.md)).
The nesting is not charged against the recursion limit in either interpreter, and a session with no memory limit has
no bound to hit.


## Experimental host coroutine dialect

A first module statement `from __monty_host__ import host_coroutines` selects a Monty compiler dialect.
This reserved directive is not an importable Python module, binds no name, and cannot be aliased or placed after another statement (including a docstring).
It applies to that source compilation only; a later REPL or `exec` compilation must select it again.

In this dialect all `async def` functions return lazy generators driven by `next`, `send`, and one-argument `throw`.
`await` delegates to a generator continuation, including another dialect async function.
When driven with `next`, `send`, or `throw`, it does not await native Coroutine, ExternalFuture, GatherFuture, or arbitrary `__await__` objects, and does not select or run sibling tasks.
The ordinary `asyncio` scheduler remains the default without the directive; the two kinds of awaitable are not interchangeable.
`await` outside an async function, in lambdas, or in comprehension bodies is rejected in this dialect.
Async generators, `async for`, and async comprehensions remain unsupported.

`async with` is available only inside dialect async functions, on simple user-class instances defining class-level `__aenter__` and `__aexit__` methods returning generator continuations (or native awaitables when using the native entry below).
Multiple managers and unpacked targets use the same nesting and binding rules as ordinary `with`.
There are no async contextmanager decorators, inheritance, or general awaitable protocol adapters.
Missing async methods raise `AttributeError`; non-instance context values raise `TypeError`.
When manually driven, invalid await operands (such as `await 3`) and async protocol methods returning non-generators raise `NotImplementedError`, rather than CPython's `TypeError`.
Exit receives the exception type, exception object and `None` rather than a traceback object.
As with [ordinary context managers](with.md), exit is looked up again at exit, so replacing `__aexit__` during the body calls the replacement.

Cancellation and resource/child ownership are host/library policy; the syntax itself injects no cancellation and implements no deadlines or task groups.
The experimental examples in `tools/coroutines/` include a combined `Timebox` resource/child scope with explicit host-driven spawn, join and exit drain requests.
They do not provide an importable scheduler package or a production authority journal.

### Experimental native entry and host cancellation

The explicit generator method `_native1()` runs the dialect continuation on the existing native task.
Direct yields and dialect `await` on that route accept native coroutines, external futures, and `asyncio.gather` results.
Generator operands still delegate to their durable continuation; arbitrary `__await__` objects remain unsupported.
Dialect generator children require an ordinary-coroutine binding that calls `_native1()`; nested gathers and external futures remain valid gather inputs.
This does not make dialect generators directly acceptable as gather children.
Ordinary `next`, `send`, and `throw` retain their separate behavior.
The experimental host cancellation input delivers a catchable `ValueError`, rather than an asyncio `CancelledError`.
Catching it does not clear the native terminal cancelled outcome, and the first host reason wins even when empty.
Compiler-marked plain `finally` bodies and async context exit delegation protect their external waits from host cancellation; ordinary operation errors remain catchable and are not masked.
A first cancellation during cleanup preserves its reply and is delivered when outermost protection ends, before ordinary body work or another request.
Pending returns yield to the catchable signal after required outer cleanup.
An escaping original or cleanup error retains precedence without a second catchable cancellation signal.
While cancellation is pending and undelivered, prospective `except` dispatch outside active cleanup is bypassed before evaluating type expressions, including handlers that would log and rethrow.
If matching began before cancellation, its unresolved error is retained across selector calls and suspension; remaining unprotected matching work is abandoned when cleanup protection ends.
A selector return is not suppression of its caller’s error.
Handlers inside active cleanup retain normal behavior; handlers after cancellation delivery may perform authored recovery.
Required synchronous and async context exit calls finish their suppression bookkeeping before delivery: a false result retains the error, a true result suppresses it, and a raised exit error replaces it.
This bookkeeping guarantee does not provide preemption of blocking synchronous I/O or nonterminating exit code.
This does not implement Python task cancellation, task groups, or automatic aggregate cleanup failures.


### Experimental owned children

`asyncio._scope()`, `_spawn(coroutine)`, `_cancel(handle)` and `_drain()` are experimental native-only authoring primitives, not asyncio compatibility APIs.
One scope may be open per Rust task; nested scopes in the same task are rejected.
`_spawn` returns an awaitable child handle; only its owning task and scope can join or cancel it.
An explicit join delivers the result or exception, consuming the scope's error obligation only when the owner resumes.
Unjoined ordinary child errors remain owned until `_drain()` returns them as exception objects, in spawn order.
Sibling failures do not cancel other children.
`_drain()` closes admission, cancels unfinished children, and waits for their cleanup before returning.
An authoring context manager must await this drain in `__aexit__` before resource cleanup.
Cancelling a completed owned handle is a no-op.
Guest child cancellation raises one catchable `ValueError('guest')`; returning after catching it still makes joining raise that cancellation.
An ordinary failure during child cleanup remains visible as an error.
Host cancellation fans out through the owned child tree, including owners parked on child joins.
A cancelled join detaches its waiter without consuming the child outcome; scope exit still drains the child before parent cleanup.
A drain in progress preserves its reply and completes before delivery of a pending cancellation.
Child admission stays closed after host cancellation, including after catching the signal or attempting a new scope.
Gathered descendants remain scope obligations after first-error delivery or after their immediate owner completes.
Scope exit cancels and drains those descendants before parent resource cleanup; first-error delivery alone does not cancel siblings.
Cancellation detaches a gather waiter without consuming an undelivered error, and reaches nested gathered tasks and direct external futures.
Gathered cleanup continuations retain their protected replies across repeated cancellation.
Ordinary gather failures that no waiter receives are retained in `scope.errors` before the explicit-child errors.
Their order follows settlement and lifetime transfer; explicit-child errors retain spawn order.
New gathers are rejected after cancellation except within protected cleanup.
The gather lifetime contract requires checkpoints produced with dump version26 or later; older snapshots retain their earlier owned-gather limitations.
The thin experimental Scope binding supports deferred async cleanup in LIFO order after child drain, retaining ordinary failures in `scope.errors` while attempting the remaining cleanups.

## Experimental bounded queues

`asyncio.Queue` is not provided. The native continuation experiment exposes
`asyncio._queue(capacity)`, `await asyncio._queue_put(queue, value)` and
`await asyncio._queue_get(queue)`. Capacity must be an integer from 1 through 4294967295 on both native and Wasm;
booleans, zero, negative and unbounded capacities are unsupported. Tokens have no public
attributes or methods. Operations are lazy and single-await; wrap an operation
in an ordinary coroutine to use it with `gather`.

Values and waiters are FIFO. Both immediate and blocked operations cooperate
with the scheduler. Cancellation before delivery revokes the reservation,
leaving a get's value in the queue and a put's value outside it. There is no
`join`, `task_done`, `shutdown`, timeout or nonblocking API. A program whose
remaining tasks all wait on queues can deadlock; there is no timer or implicit
producer. Checkpoints are available at external suspension boundaries, not
at every queue await.
