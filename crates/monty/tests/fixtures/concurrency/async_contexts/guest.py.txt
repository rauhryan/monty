# ruff: noqa: F401
from __monty_host__ import host_coroutines


def effect(payload):
    """One host descriptor; its continuation is driven with send/throw."""
    return (yield payload)


class Timebox:
    """Sequential resource scope: metadata and lazy descriptors, no clock loop."""

    def __init__(self, events, failures, deadline, suppress=False):
        self.events = events
        self.failures = failures
        self.deadline = deadline
        self.suppress = suppress
        self.deferred = []

    async def __aenter__(self):
        self.events.append(['enter', self.deadline])
        await effect(['scope-enter', self.deadline])
        return self

    def defer(self, operation, handle):
        self.deferred.append([operation, handle])

    async def __aexit__(self, typ, value, traceback):
        self.events.append(['exit', None if typ is None else str(value)])
        cursor = len(self.deferred)
        while cursor > 0:
            cursor -= 1
            try:
                await effect(self.deferred[cursor])
            except Exception as error:
                self.failures.append(str(error))
        return self.suppress


async def cart(events, failures, mode):
    async with Timebox(events, failures, 50, mode == 'suppress') as tg:
        hold = await effect(['acquire'])
        tg.defer('release', hold)
        tg.defer('audit-release', hold)
        if mode == 'return':
            return hold
        if mode == 'error' or mode == 'suppress':
            raise ValueError('body-failed')
        try:
            return await effect(['use', hold])
        except ValueError:
            if mode == 'catch':
                return 'caught'
            raise
    return 'suppressed'
