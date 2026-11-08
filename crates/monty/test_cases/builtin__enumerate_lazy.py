import itertools


events = []


class Start:
    def __index__(self):
        events.append('index')
        return 2**100


class Source:
    def __init__(self):
        self.n = 0

    def __iter__(self):
        events.append('iter')
        return self

    def __next__(self):
        self.n += 1
        events.append('next')
        if self.n == 1:
            raise ValueError('retry')
        if self.n == 3:
            raise StopIteration
        return self.n


source = Source()
e = enumerate(source, Start())
assert events == ['index', 'iter']
assert iter(e) is e
try:
    next(e)
except ValueError as error:
    assert str(error) == 'retry'
else:
    assert False
assert next(e) == (2**100, 2)
assert next(e, 'done') == 'done'
# Like CPython, enumerate asks the source again after exhaustion.
assert next(e) == (2**100 + 1, 4)
assert events == ['index', 'iter', 'next', 'next', 'next', 'next']
items = [1]
e = enumerate(items)
items.append(2)
assert list(e) == [(0, 1), (1, 2)]
assert list(e) == []
assert list(enumerate([1, 2], 2**63 - 1)) == [(2**63 - 1, 1), (2**63, 2)]
e = enumerate(itertools.count(), -2)
assert [next(e), next(e), next(e)] == [(-2, 0), (-1, 1), (0, 2)]


def values():
    yield 4
    yield 5


assert list(enumerate(values())) == [(0, 4), (1, 5)]
try:
    enumerate(1, start=1.5)
except TypeError as error:
    assert str(error) == "'float' object cannot be interpreted as an integer"
else:
    assert False
