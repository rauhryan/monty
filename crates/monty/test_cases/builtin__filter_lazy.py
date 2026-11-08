# Acquisition is eager, but source advancement and predicate calls are lazy.
events = []
error = ValueError('predicate')


class Source:
    def __init__(self):
        self.n = 0

    def __iter__(self):
        events.append('iter')
        return self

    def __next__(self):
        self.n += 1
        events.append(self.n)
        if self.n == 5:
            raise error
        if self.n > 6:
            raise StopIteration
        return self.n


def predicate(item):
    events.append(-item)
    if item == 2:
        raise error
    return item % 2 == 0


f = filter(predicate, Source())
assert events == ['iter']
assert iter(f) is f
try:
    next(f)
except ValueError as caught:
    assert caught is error
else:
    assert False
assert events == ['iter', 1, -1, 2, -2]
assert next(f) == 4
try:
    next(f)
except ValueError as caught:
    assert caught is error
else:
    assert False
assert next(f) == 6
assert next(f, None) is None
assert list(filter(None, [0, '', [], 1, 'a'])) == [1, 'a']
assert list(filter(bool, [0, 1, 2])) == [1, 2]
assert list(filter(42, [])) == []
f = filter(42, [1])
try:
    next(f)
except TypeError:
    pass
else:
    assert False
assert next(f, None) is None

# Predicate StopIteration preserves identity and does not close the iterator.
stop = StopIteration('predicate')


def sometimes_stop(item):
    if item == 1:
        raise stop
    return True


f = filter(sometimes_stop, [1, 2])
try:
    next(f)
except StopIteration as caught:
    assert caught is stop
else:
    assert False
assert next(f) == 2

items = [1]
f = filter(None, items)
items.append(2)
assert list(f) == [1, 2]


def infinite():
    n = 0
    while True:
        yield n
        n += 1


f = filter(lambda n: n % 2, infinite())
assert [next(f) for _ in range(3)] == [1, 3, 5]
