from __monty_host__ import host_coroutines

MARKERS.clear()

def operation(name):
    return (yield exchange(name))

def fold(left, right):
    assert left is TOKEN and right is TOKEN
    MARKERS.append('fold')
    return TOKEN

mixed_child = legacy_generator(TOKEN)

def mixed():
    yield from mixed_child

mixed_generator = mixed()
assert next(mixed_generator) is TOKEN
MARKERS.clear()

async def current(mode, route):
    if mode == 'delivered':
        try:
            await operation('body')
        except ValueError as error:
            assert str(error) == 'FIRST'
            await operation('recovery')
            await activate(route)
    else:
        try:
            pass
        finally:
            await operation('cleanup')
            await activate(route)
    return 'ok'

async def activate(route):
    if route == 'frozen':
        assert functools.reduce(fold, [TOKEN, TOKEN]) is TOKEN
    elif route == 'coroutine':
        assert await legacy_coroutine(TOKEN) == 'legacy-ok'
    elif route == 'new':
        assert next(legacy_new) is TOKEN
    elif route == 'dormant':
        assert legacy_dormant.send(TOKEN) is TOKEN
    elif route == 'mixed':
        # The current delegate starts before encountering the old saved segment.
        assert mixed_generator.send(TOKEN) is TOKEN
    MARKERS.append('ran')
