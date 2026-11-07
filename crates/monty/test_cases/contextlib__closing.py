from contextlib import closing


events = []


class Resource:
    def close(self):
        events.append('closed')


resource = Resource()
manager = closing(thing=resource)
with manager as entered:
    assert entered is resource
assert events == ['closed']
body_error = ValueError('body')
try:
    with manager:
        raise body_error
except ValueError as caught:
    assert caught is body_error
else:
    assert False
assert events == ['closed', 'closed']

close_error = RuntimeError('close')


class Failing:
    def close(self):
        raise close_error


try:
    with closing(Failing()):
        raise body_error
except RuntimeError as caught:
    assert caught is close_error
    assert caught.__context__ is body_error
else:
    assert False


def values():
    try:
        yield 1
    finally:
        events.append('generator closed')


with closing(values()) as generator:
    assert next(generator) == 1
assert events[-1] == 'generator closed'


# close is looked up at exit, and its return value does not suppress an error.
class Truthy:
    def close(self):
        return True


try:
    with closing(Truthy()):
        raise body_error
except ValueError as caught:
    assert caught is body_error
else:
    assert False
