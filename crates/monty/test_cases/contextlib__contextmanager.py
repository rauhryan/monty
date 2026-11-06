from contextlib import contextmanager


@contextmanager
def resource(name, *, enabled=True):
    yield (name, enabled)


with resource('test', enabled=False) as value:
    assert value == ('test', False)


def raw():
    yield 'keyword'


with contextmanager(func=raw)() as value:
    assert value == 'keyword'

try:
    contextmanager()
except TypeError as error:
    assert str(error) == "contextmanager() missing 1 required positional argument: 'func'"
else:
    assert False
