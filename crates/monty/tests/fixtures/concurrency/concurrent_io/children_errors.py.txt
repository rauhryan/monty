from __monty_host__ import host_coroutines
import json

TRACE = []


def request(label):
    return (yield exchange(label))


async def operation(label):
    await request(label)
    if label == 'A':
        raise ValueError('child-error')
    if label == 'B':
        raise ValueError('other-error')
    TRACE.append('sibling-completed')
    return 'beta'


async def parent():
    scope = Scope()
    async with scope:
        first = scope.spawn(operation('A'))
        scope.spawn(operation('B'))
        sibling = scope.spawn(operation('C'))
        try:
            await first
        except ValueError as error:
            joined = str(error)
        await sibling
    return json.dumps(
        {
            'joined_error': joined,
            'unconsumed_at_exit': [str(error) for error in scope.errors],
            'trace': TRACE,
        }
    )


parent()._native1()
