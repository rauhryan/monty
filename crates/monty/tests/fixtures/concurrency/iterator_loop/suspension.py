# ruff: noqa: F821
import asyncio
import json


class Iterator:
    def __init__(self, token, pending, events, label):
        self.token = token
        self.pending = pending
        self.events = events
        self.label = label
        self.count = 0

    def __iter__(self):
        self.events.append('iter')
        self.pending.append(exchange(self.label + '-iter'))
        return self

    def __next__(self):
        self.count += 1
        self.events.append(self.count)
        self.pending.append(exchange(self.label + '-next-' + str(self.count)))
        if self.count == 2:
            raise StopIteration(self.token)
        return self.token

    def close(self):
        assert False, 'for must not close the iterator'


class Invalid:
    def __init__(self, pending):
        self.pending = pending

    def __iter__(self):
        self.pending.append(exchange('invalid-iter'))
        return 42


class Failing:
    def __init__(self, pending, error):
        self.pending = pending
        self.error = error

    def __iter__(self):
        return self

    def __next__(self):
        self.pending.append(exchange('failing-next'))
        raise self.error


async def main():
    token = []
    token.append(token)
    pending = []
    events = []
    for item in Iterator(token, pending, events, 'loop'):
        assert item is token
    else:
        events.append('else')
    assert events == ['iter', 1, 2, 'else']
    assert await asyncio.gather(*pending) == ['ok', 'ok', 'ok']
    pending.clear()
    events.clear()
    values = [item for item in Iterator(token, pending, events, 'comp')]
    assert len(values) == 1 and values[0] is token
    assert events == ['iter', 1, 2]
    assert await asyncio.gather(*pending) == ['ok', 'ok', 'ok']
    pending.clear()
    try:
        for item in Invalid(pending):
            assert False
    except TypeError as error:
        assert str(error) == "iter() returned non-iterator of type 'int'"
    else:
        assert False
    assert await pending.pop() == 'ok'
    error = ValueError(token)
    try:
        for item in Failing(pending, error):
            assert False
    except ValueError as caught:
        assert caught is error and caught.args[0] is token
    else:
        assert False
    assert await pending.pop() == 'ok'
    return json.dumps('ok')


asyncio.run(main())
