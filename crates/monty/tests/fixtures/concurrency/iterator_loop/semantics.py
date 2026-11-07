events = []
token = []


class Iterator:
    def __init__(self, count):
        self.count = count

    def __iter__(self):
        events.append('iter')
        return self

    def __next__(self):
        events.append(self.count)
        if self.count == 0:
            raise StopIteration(token)
        self.count -= 1
        return token

    def close(self):
        assert False, 'for must not close the iterator'


for value in Iterator(2):
    assert value is token
else:
    events.append('else')
assert events == ['iter', 2, 1, 0, 'else']
events.clear()
iterator = Iterator(2)
for value in iterator:
    break
else:
    assert False
assert events == ['iter', 2]
assert next(iterator) is token
events.clear()
error = StopIteration(token)
try:
    for value in Iterator(1):
        raise error
except StopIteration as caught:
    assert caught is error
else:
    assert False
assert events == ['iter', 1]
events.clear()
assert [value for value in Iterator(2)] == [token, token]
assert events == ['iter', 2, 1, 0]
events.clear()
for outer in Iterator(2):
    for inner in Iterator(2):
        continue
    else:
        events.append('inner-else')
assert events == [
    'iter',
    2,
    'iter',
    2,
    1,
    0,
    'inner-else',
    1,
    'iter',
    2,
    1,
    0,
    'inner-else',
    0,
]


class Failing:
    def __iter__(self):
        return self

    def __next__(self):
        raise error


error = ValueError(token)
try:
    for value in Failing():
        assert False
except ValueError as caught:
    assert caught is error and caught.args[0] is token
else:
    assert False
iterator = value = error = None
None
