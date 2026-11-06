# ruff: noqa: F821
import asyncio
import json
from contextlib import contextmanager


def sequence(token, pending, label):
    pending.append(exchange(label))
    yield token
    yield token


class Iterator:
    def __init__(self, token, pending, label):
        self.label = label
        self.token = token
        self.pending = pending

    def __iter__(self):
        self.pending.append(exchange(self.label + '-iter'))
        return self

    def __next__(self):
        self.pending.append(exchange(self.label + '-next'))
        return self.token

    def throw(self, error):
        self.pending.append(exchange(self.label + '-throw'))
        raise StopIteration(self.token)

    def close(self):
        self.pending.append(exchange(self.label + '-close'))


def delegate(iterator):
    return (yield from iterator)


@contextmanager
def manager(token, pending):
    pending.append(exchange('enter'))
    try:
        yield token
    finally:
        pending.append(exchange('exit'))


def invalid_completion(token):
    if False:
        yield
    raise StopIteration(token)


async def main():
    token = []
    token.append(token)
    pending = []
    assert list(sequence(token, pending, 'collect-list'))[0] is token
    assert await pending.pop() == 'ok'
    assert tuple(sequence(token, pending, 'collect-tuple'))[1] is token
    assert await pending.pop() == 'ok'
    gen = delegate(Iterator(token, pending, 'throwing'))
    assert next(gen) is token
    assert await asyncio.gather(*pending) == ['ok', 'ok']
    pending.clear()
    try:
        gen.throw(ValueError(token))
    except StopIteration as done:
        assert done.value is token
    else:
        assert False
    assert await pending.pop() == 'ok'
    gen = delegate(Iterator(token, pending, 'closing'))
    assert next(gen) is token
    gen.close()
    assert await asyncio.gather(*pending) == ['ok', 'ok', 'ok']
    pending.clear()
    error = StopIteration(token)
    try:
        with manager(token, pending) as value:
            assert value is token
            raise error
    except StopIteration as caught:
        assert caught is error
    else:
        assert False
    assert await asyncio.gather(*pending) == ['ok', 'ok']
    try:
        next(invalid_completion(token))
    except RuntimeError as converted:
        assert str(converted) == 'generator raised StopIteration'
        assert await exchange('cause') == 'ok'
    else:
        assert False
    return json.dumps('ok')


asyncio.run(main())
