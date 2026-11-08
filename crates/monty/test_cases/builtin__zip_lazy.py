import itertools


events = []


class Source:
    def __init__(self, name, items):
        self.name = name
        self.items = iter(items)

    def __iter__(self):
        events.append(self.name + '-iter')
        return self

    def __next__(self):
        events.append(self.name + '-next')
        item = next(self.items)
        if isinstance(item, BaseException):
            raise item
        return item


error = ValueError('retry')
a = Source('a', [1, 2, 3, 4])
b = Source('b', [error, 10, 20])
z = zip(a, b)
assert iter(z) is z
assert events == ['a-iter', 'b-iter']
try:
    next(z)
except ValueError as caught:
    assert caught is error
else:
    assert False
assert events == ['a-iter', 'b-iter', 'a-next', 'b-next']
assert next(z) == (2, 10)
assert next(z) == (3, 20)
assert next(z, None) is None
assert list(zip()) == []
assert next(zip(strict=True), 'done') == 'done'
items = [1]
z = zip(items, itertools.count())
items.append(2)
assert list(z) == [(1, 0), (2, 1)]
stop = StopIteration('marker')
z = zip(Source('x', [stop, 1]), [2, 3])
try:
    next(z)
except StopIteration as caught:
    assert caught is stop
else:
    assert False
assert next(z) == (1, 2)
for inputs, message in [
    (([1], []), 'zip() argument 2 is shorter than argument 1'),
    (([], [1]), 'zip() argument 2 is longer than argument 1'),
    (([1], [2], []), 'zip() argument 3 is shorter than arguments 1-2'),
    (([], [], [1]), 'zip() argument 3 is longer than arguments 1-2'),
]:
    z = zip(*inputs, strict=True)
    try:
        next(z)
    except ValueError as caught:
        assert str(caught) == message
        assert caught.__context__ is None and caught.__cause__ is None
    else:
        assert False
z = zip(Source('x', [stop, 1]), [2, 3], strict=True)
try:
    next(z)
except ValueError:
    pass
else:
    assert False
assert next(z) == (1, 3)
# Strict probing propagates other errors, and retry starts from the first input.
z = zip(Source('x', [stop, 7]), Source('y', [error, 8]), strict=True)
try:
    next(z)
except ValueError as caught:
    assert caught is error
else:
    assert False
assert next(z) == (7, 8)
shared = iter([1, 2, 3, 4])
assert list(zip(shared, shared, strict=True)) == [(1, 2), (3, 4)]


def values():
    yield 9


assert list(zip(values(), [8], strict=1)) == [(9, 8)]
