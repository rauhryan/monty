from __monty_host__ import host_coroutines
import json


def request(label):
    return (yield exchange(label))


async def operation(label):
    try:
        return await request(label)
    except ValueError:
        return 'caught-but-cancelled'


async def parent():
    scope = Scope()
    async with scope:
        first = scope.spawn(operation('A'))
        sibling = scope.spawn(operation('B'))
        await request('cancel')
        scope.cancel(first)
        try:
            await first
        except ValueError as error:
            cancelled = ['cancelled', str(error)]
        result = await sibling
        scope.cancel(sibling)
    return json.dumps(
        {
            'cancelled_join': cancelled,
            'sibling_result': result,
            'completed_cancel': 'no-op',
            'errors': [str(error) for error in scope.errors],
        }
    )


parent()._native1()
