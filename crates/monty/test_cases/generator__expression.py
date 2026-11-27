events = []


def source():
    events.append('source')
    return [1, 2, 3]


def keep(value):
    events.append(value)
    return value > 1


g = (value for value in source() if keep(value))
assert events == ['source']
assert next(g) == 2
assert events == ['source', 1, 2]
assert next(g) == 3
assert next(g, 'done') == 'done'


class Iterator:
    def __init__(self):
        self.calls = 0
        self.value = 0

    def __iter__(self):
        self.calls += 1
        return self

    def __next__(self):
        self.value += 1
        if self.value > 2:
            raise StopIteration
        return self.value


it = Iterator()
g = (x for x in it)
assert it.calls == 1
assert list(g) == [1, 2]
assert it.calls == 1


def enclosing():
    offset = 10
    g = (x + offset for x in [1, 2])
    offset = 20
    assert next(g) == 21
    return g


assert list(enclosing()) == [22]
assert list((x, y) for x in [1, 2] for y in range(x) if y == 0) == [(1, 0), (2, 0)]
assert list(a + b for a, b in [(1, 2), (3, 4)]) == [3, 7]
assert list((x for x in [y, y + 1]) for y in []) == []
assert [list(g) for g in [(x for _ in [0]) for x in [1, 2]]] == [[2], [2]]


def assignment():
    last = None
    g = ((last := x) for x in [1, 2])
    assert last is None
    assert next(g) == 1
    assert last == 1
    assert list(g) == [2]
    assert last == 2


assignment()
last = None
g = ((last := x) for x in [3])
assert last is None
assert list(g) == [3]
assert last == 3

g = (1 // x for x in [1, 0])
assert next(g) == 1
try:
    next(g)
except ZeroDivisionError:
    pass
else:
    assert False
assert next(g, 'closed') == 'closed'

try:
    g = (x for x in 3)
except TypeError:
    pass
else:
    assert False, 'first iterator acquisition must be eager'

g = (x for x in range(1000000000))
assert next(g) == 0
g.close()
assert next(g, 'closed') == 'closed'

# Consumers must retain lazy evaluation and stop at the decisive item.
events = []


def observed(x):
    events.append(x)
    return x


assert any(observed(x) for x in [0, 2, 3])
assert events == [0, 2]
events.clear()
assert not all(observed(x) for x in [1, 0, 3])
assert events == [1, 0]
assert sum(x for x in range(5)) == 10

# Native consumers advance lazily without retaining a native stack between calls.
import math

assert math.fsum(x for x in [1e100, 1, -1e100]) == 1
assert dict((x, x + 1) for x in [1, 2]) == {1: 2, 2: 3}
assert set(x for x in [1, 1, 2]) == {1, 2}
assert sorted(x for x in [3, 1, 2]) == [1, 2, 3]
assert ''.join(str(x) for x in [1, 2]) == '12'
assert min(x for x in [3, 1, 2]) == 1
assert max((x for x in []), default=9) == 9

# Acquiring an invalid iterator precedes rejecting the start argument.
try:
    sum(1, '')
except TypeError as error:
    assert str(error) == "'int' object is not iterable"
else:
    assert False
