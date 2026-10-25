"""CPython-only cooperative contract model; no VM, persistence or CPU preemption.

One owning scope, one root and direct children deliberately bound this spike.
Gather collect-all and single-consumer joins are PROPOSALS, not selected policy.
"""

from collections import deque
from dataclasses import dataclass, field


class Cancelled(Exception):
    """Catchable cancellation whose task outcome remains latched."""


class Aggregate(Exception):
    """Proposed ordered child failure representation for this reference only."""


@dataclass
class Request:
    """Platform awaitable: only this closed command vocabulary may suspend."""

    kind: str
    value: object = None

    def __await__(self):
        return (yield self)


@dataclass
class Task:
    """In-memory activation and explicit delivery ownership."""

    coro: object
    wait: object = None
    reply: object = None
    error: object = None
    done: bool = False
    result: object = None
    failure: object = None
    cancel: object = None
    protected: bool = False
    consumed: bool = False
    claimed: bool = False
    grant: object = None
    delivery: list = field(default_factory=list)


class Semaphore:
    """FIFO internal wait with a reserved permit until actual task delivery."""

    def __init__(self, runtime, permits):
        self.runtime = runtime
        self.permits = permits
        self.waiters = deque()

    async def __aenter__(self):
        await Request('acquire', self)
        return self

    async def __aexit__(self, *_):
        self.release()

    def release(self):
        """Reserve for the oldest waiter; cancellation returns an undelivered grant."""
        if self.waiters:
            tid = self.waiters.popleft()
            self.runtime.tasks[tid].grant = self
            self.runtime.wake(tid)
        else:
            self.permits += 1


class Resource:
    """Explicitly protected yielding cleanup, attempting both cleanup contracts."""

    def __init__(self, runtime, name):
        self.runtime, self.name = runtime, name

    async def __aenter__(self):
        return self

    async def __aexit__(self, *_):
        task = self.runtime.tasks[self.runtime.current]
        task.protected = True
        for verb in ('audit', 'close'):
            try:
                await Request('op', f'{verb}:{self.name}')
            except Exception as error:
                self.runtime.cleanup_failures.append(str(error))
        # This resource encloses the whole child body in this bounded model.


class Scope:
    """Single root scope: close admission, cancel/drain, then parent cleanup."""

    def __init__(self, runtime):
        self.runtime = runtime

    async def __aenter__(self):
        return self

    async def __aexit__(self, typ, value, traceback):
        r = self.runtime
        r.closed = True
        r.tasks[0].protected = True
        for tid in list(r.tasks)[1:]:
            r.cancel_task(tid, 'scope-exit')
        await Request('drain', list(r.tasks)[1:])
        try:
            await Request('op', 'close:parent')
        except Exception as error:
            r.cleanup_failures.append(str(error))
        errors = [str(t.failure) for tid, t in r.tasks.items() if tid and t.failure and not t.consumed]
        if errors or r.cleanup_failures:
            original = [str(value)] if typ is not None and not isinstance(value, Cancelled) else []
            raise Aggregate(original + errors + r.cleanup_failures)


class Runtime:
    """Deterministic FIFO advancement between host boundaries, with ordered inputs."""

    def __init__(self, body, generation='G1'):
        self.generation = generation
        self.revision = 0
        self.tasks = {}
        self.ready = deque()
        self.pending = {}
        self.events = set()
        self.next_op = 0
        self.current = None
        self.closed = False
        self.cleanup_failures = []
        self.trace = []
        self.deadline = None  # Host sets absolute time once; no guest clock reads.
        self.spawn(body(self))

    def spawn(self, coro):
        """Allocate monotonic task IDs; reject admission before publishing a child."""
        if self.closed:
            coro.close()
            raise ValueError('scope-closed')
        tid = len(self.tasks)
        self.tasks[tid] = Task(coro)
        self.ready.append(tid)
        return tid

    async def gather(self, *coros):
        """Proposed collect-all join; results and errors follow input order."""
        tids = [self.spawn(coro) for coro in coros]
        return await Request('join', tids)

    def wake(self, tid, value=None, error=None):
        """Queue once; values/errors are delivered only when selected."""
        task = self.tasks[tid]
        task.wait, task.reply, task.error = None, value, error
        if tid not in self.ready:
            self.ready.append(tid)

    def cancel_task(self, tid, reason):
        """First cancellation wins; detach waits without consuming queued joins."""
        task = self.tasks[tid]
        if task.done or task.cancel:
            return
        task.cancel = reason
        if task.protected:
            return
        if task.grant:
            task.grant.release()
            task.grant = None
        if task.wait and task.wait.kind == 'acquire':
            task.wait.value.waiters.remove(tid)
        for oid, op in list(self.pending.items()):
            if op['task'] == tid:
                del self.pending[oid]
        for child in task.delivery:
            self.tasks[child].claimed = False
        task.delivery = []
        if task.wait and task.wait.kind == 'join':
            for child in task.wait.value:
                self.tasks[child].claimed = False
        self.wake(tid, error=Cancelled(reason))

    def advance(self, events=(), budget=100):
        """Apply input order before FIFO work; budget counts cooperative resumes only."""
        if budget < 0:
            raise ValueError('negative budget')
        for event in events:
            self.accept(event)
        for _ in range(budget):
            if not self.ready:
                break
            tid = self.ready.popleft()
            self.current = tid
            task = self.tasks[tid]
            task.grant = None  # Permit now belongs to the resumed async-with body.
            for child in task.delivery:
                self.tasks[child].consumed = True
            task.delivery = []
            value, error = task.reply, task.error
            task.reply = task.error = None
            try:
                request = task.coro.throw(error) if error else task.coro.send(value)
            except StopIteration as stop:
                task.done, task.result = True, stop.value
            except Exception as failure:
                task.done = True
                if not isinstance(failure, Cancelled):
                    task.failure = failure
            else:
                self.park(tid, request)
            self.wake_joins()
        self.revision += 1
        return self.boundary()

    def accept(self, event):
        """Reject duplicate/stale inputs without guest advancement from that input."""
        eid, generation, kind, target, value = event
        if generation != self.generation or eid in self.events:
            self.trace.append(f'reject:{eid}')
            return
        self.events.add(eid)
        if kind == 'cancel' and target == 'S0':
            self.closed = True
            for tid in self.tasks:
                self.cancel_task(tid, value)
        elif kind == 'deadline':
            if self.deadline is None:
                self.deadline = value
        elif kind in ('value', 'error') and target in self.pending:
            op = self.pending.pop(target)
            self.wake(
                op['task'],
                value=value if kind == 'value' else None,
                error=ValueError(value) if kind == 'error' else None,
            )
        else:
            self.trace.append(f'reject:{eid}')

    def park(self, tid, request):
        """Interpret one platform suspension, never execute host I/O here."""
        task = self.tasks[tid]
        task.wait = request
        if request.kind == 'op':
            oid = f'{self.generation}:O{self.next_op}'
            self.next_op += 1
            self.pending[oid] = {'task': tid, 'scope': 'S0', 'contract': request.value, 'version': 1}
            self.trace.append(request.value)
        elif request.kind == 'acquire':
            sem = request.value
            if sem.permits:
                sem.permits -= 1
                task.grant = sem
                self.wake(tid)
            else:
                sem.waiters.append(tid)
        elif request.kind == 'join':
            if len(set(request.value)) != len(request.value) or any(
                self.tasks[c].claimed or self.tasks[c].consumed for c in request.value
            ):
                self.wake(tid, error=ValueError('join-consumed-or-claimed'))
            else:
                for child in request.value:
                    self.tasks[child].claimed = True
        elif request.kind != 'drain':
            raise ValueError('unknown request')

    def wake_joins(self):
        """Reserve outcomes on wake; consume only at actual continuation delivery."""
        for tid, task in self.tasks.items():
            req = task.wait
            if not req or req.kind not in ('join', 'drain'):
                continue
            if not all(self.tasks[c].done for c in req.value):
                continue
            children = [self.tasks[c] for c in req.value]
            if req.kind == 'join':
                task.delivery = list(req.value)
                errors = [str(c.failure) if c.failure else c.cancel for c in children if c.failure or c.cancel]
                self.wake(tid, [c.result for c in children], Aggregate(errors) if errors else None)
            else:
                self.wake(tid)

    def boundary(self):
        """Projection is repeatable but is NOT a restorable snapshot."""
        root = self.tasks[0]
        status = 'runnable' if self.ready else 'blocked' if self.pending else 'failed'
        outcome = None
        if root.done:
            status = 'complete'
            outcome = (
                ('cancelled', root.cancel)
                if root.cancel
                else (('error', str(root.failure)) if root.failure else ('value', root.result))
            )
        return {
            'status': status,
            'generation': self.generation,
            'revision': self.revision,
            'pending': dict(self.pending),
            'outcome': outcome,
            'cleanup_failures': list(self.cleanup_failures),
            'deadline': self.deadline,
            'diagnostic': 'internal-deadlock' if status == 'failed' else None,
        }


async def read_one(runtime, limit, name):
    """One vertical workload: internal permit, external read, protected cleanup."""
    async with limit:
        async with Resource(runtime, name):
            try:
                return await Request('op', f'read:{name}')
            except Cancelled:
                runtime.trace.append(f'caught:{name}')
                return 'caught'


async def workload(runtime):
    """Three reads share two permits; parent cleanup follows all child cleanup."""
    runtime.limit = Semaphore(runtime, 2)
    async with Scope(runtime):
        return await runtime.gather(*(read_one(runtime, runtime.limit, name) for name in ('A', 'B', 'C')))
