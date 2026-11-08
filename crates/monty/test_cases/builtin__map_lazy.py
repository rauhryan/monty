events = []
error = ValueError('callback')


class Source:
    def __init__(self, name):
        self.name = name
        self.n = 0

    def __iter__(self):
        events.append(self.name)
        return self

    def __next__(self):
        self.n += 1
        events.append(self.name + str(self.n))
        if self.n == 4:
            raise StopIteration
        return self.n


def combine(a, b):
    events.append('callback')
    if a == 1:
        raise error
    return a + b


m = map(combine, Source('a'), Source('b'))
assert events == ['a', 'b']
assert iter(m) is m
try:
    next(m)
except ValueError as caught:
    assert caught is error
else:
    assert False
assert events == ['a', 'b', 'a1', 'b1', 'callback']
assert next(m) == 4
assert next(m) == 6
assert next(m, None) is None
assert events[-1] == 'a4'
assert list(map(None, [])) == []
m = map(None, [1, 2])
for _ in range(2):
    try:
        next(m)
    except TypeError:
        pass
    else:
        assert False
assert next(m, None) is None
stop = StopIteration('callback')


def callback(item):
    if item == 1:
        raise stop
    return item


m = map(callback, [1, 2])
try:
    next(m)
except StopIteration as caught:
    assert caught is stop
else:
    assert False
assert next(m) == 2
values = [1]
m = map(lambda x: x * 2, values)
values.append(2)
assert list(m) == [2, 4]


def infinite():
    while True:
        yield 1


assert list(map(lambda a, b: a + b, infinite(), [2, 3])) == [3, 4]
assert list(map(lambda a, b, c: a + b + c, [1], [2], [3])) == [6]
