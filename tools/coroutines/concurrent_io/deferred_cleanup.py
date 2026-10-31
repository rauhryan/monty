from __monty_host__ import host_coroutines
import json

TRACE = []


def request(label):
    return (yield exchange(label))


async def first():
    TRACE.append(['cleanup', 'first'])
    TRACE.append(['reply', await request('first')])


async def second():
    TRACE.append(['cleanup', 'second'])
    TRACE.append(['reply', await request('second')])
    raise ValueError('second-failure')


async def parent():
    scope = Scope()
    async with scope:
        scope.defer(first)
        scope.defer(second)
        try:
            await request('body')
        except ValueError as error:
            TRACE.append(['caught', str(error)])
            try:
                scope.spawn(first())
            except ValueError as closed:
                TRACE.append(['admission', str(closed)])
    try:
        async with Scope():
            TRACE.append(['reopened'])
    except ValueError as closed:
        TRACE.append(['scope', str(closed)])
    return json.dumps({'trace': TRACE, 'errors': [str(e) for e in scope.errors]})


parent()._native1()
