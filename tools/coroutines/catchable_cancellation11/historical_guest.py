import functools

MARKERS = []
TOKEN = []
TOKEN.append(TOKEN)
# Populate the genuine historical frozen registry before serialization.
assert functools.reduce(lambda a, b: a + b, [1, 2]) == 3

async def legacy_coroutine(token):
    MARKERS.append('legacy-coroutine')
    assert token is TOKEN
    return 'legacy-ok'

def legacy_generator(token):
    MARKERS.append('legacy-generator')
    received = yield token
    assert received is token
    MARKERS.append('legacy-resumed')
    yield token

legacy_new = legacy_generator(TOKEN)
legacy_dormant = legacy_generator(TOKEN)
assert next(legacy_dormant) is TOKEN
MARKERS.clear()
None
