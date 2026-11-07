import asyncio
from contextlib import contextmanager


def sequence():
    yield token
    yield token


token = []
assert list(sequence()) == [token, token]
assert tuple(sequence()) == (token, token)
assert list(sequence())[0] is token
exhausted = sequence()
assert len(list(exhausted)) == 2
assert tuple(exhausted) == ()


def delegate(items):
    return (yield from items)


for items in ([1, 2], (1, 2), range(1, 3), iter([1, 2])):
    gen = delegate(items)
    assert next(gen) == 1
    assert gen.send(None) == 2
    try:
        next(gen)
    except StopIteration as done:
        assert done.value is None
    else:
        assert False

gen = delegate([1, 2])
assert next(gen) == 1
try:
    gen.send(3)
except AttributeError:
    pass
else:
    assert False


class MissingProtocol:
    throw = None
    close = None

    def __iter__(self):
        return self

    def __next__(self):
        return 1


for action in ('throw', 'close'):
    gen = delegate(MissingProtocol())
    next(gen)
    try:
        if action == 'throw':
            gen.throw(ValueError('body'))
        else:
            gen.close()
    except TypeError:
        pass
    else:
        assert False

original = ValueError('body')
gen = delegate([1, 2])
assert next(gen) == 1
try:
    gen.throw(original)
except ValueError as caught:
    assert caught is original
else:
    assert False


class Protocol:
    def __iter__(self):
        return self

    def __next__(self):
        return 1

    def send(self, value):
        return value

    def throw(self, error):
        assert error is original
        raise StopIteration(token)

    def close(self):
        events.append('close')


events = []
gen = delegate(Protocol())
assert next(gen) == 1
assert gen.send(token) is token
try:
    gen.throw(original)
except StopIteration as done:
    assert done.value is token
else:
    assert False
gen = delegate(Protocol())
assert next(gen) == 1
gen.close()
assert events == ['close']


@contextmanager
def manager(suppress=False):
    events.append('enter')
    try:
        yield token
    except BaseException:
        if not suppress:
            raise
    finally:
        events.append('exit')


with manager() as value:
    assert value is token
with manager(True):
    raise original
for original in (
    ValueError(token),
    RuntimeError(token),
    StopIteration(token),
    GeneratorExit(token),
    asyncio.CancelledError(token),
):
    try:
        with manager():
            raise original
    except BaseException as caught:
        assert caught is original
    else:
        assert False

replacement = RuntimeError('generator raised StopIteration')


@contextmanager
def replace():
    try:
        yield
    finally:
        raise replacement


try:
    with replace():
        raise StopIteration(token)
except RuntimeError as caught:
    assert caught is replacement
else:
    assert False


@contextmanager
def empty():
    if False:
        yield


@contextmanager
def extra():
    yield
    yield


@contextmanager
def extra_throw():
    try:
        yield
    except ValueError:
        yield


for factory, message in ((empty, "generator didn't yield"), (extra, "generator didn't stop")):
    try:
        with factory():
            pass
    except RuntimeError as caught:
        assert str(caught) == message
    else:
        assert False
try:
    with extra_throw():
        raise ValueError('body')
except RuntimeError as caught:
    assert str(caught) == "generator didn't stop after throw()"
else:
    assert False


@manager()
def decorated(value):
    return value


assert decorated(token) is token
assert decorated(token) is token


def broken():
    yield token
    raise original


for collect in (list, tuple):
    try:
        collect(broken())
    except BaseException as caught:
        assert caught is original
    else:
        assert False

gen = exhausted = items = value = original = replacement = broken = collect = None
manager = replace = empty = extra = extra_throw = decorated = Protocol = None
sequence = delegate = factory = events = None
None
