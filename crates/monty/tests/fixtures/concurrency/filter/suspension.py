# ruff: noqa: F821
import asyncio
import json


async def main():
    token = []
    token.append(token)
    error = ValueError(token)
    pending = []

    class Source:
        def __init__(self):
            self.n = 0

        def __iter__(self):
            pending.append(exchange('iter'))
            return self

        def __next__(self):
            self.n += 1
            pending.append(exchange('next-' + str(self.n)))
            if self.n == 3:
                raise StopIteration
            return token

    count = 0

    def predicate(value):
        nonlocal count
        count += 1
        assert value is token
        pending.append(exchange('predicate-' + str(count)))
        if count == 1:
            raise error
        return True

    f = filter(predicate, Source())
    assert await pending.pop() == 'ok'
    try:
        next(f)
    except ValueError as caught:
        assert caught is error
    else:
        assert False
    for future in pending:
        assert await future == 'ok'
    pending.clear()
    assert next(f) is token
    for future in pending:
        assert await future == 'ok'
    pending.clear()
    assert next(f, None) is None
    assert await pending.pop() == 'ok'
    return json.dumps('ok')


asyncio.run(main())
