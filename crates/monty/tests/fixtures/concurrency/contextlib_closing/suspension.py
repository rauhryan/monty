# ruff: noqa: F821
import asyncio
import json
from contextlib import closing


async def main():
    token = []
    token.append(token)
    events = []
    pending = []

    class Resource:
        def close(self):
            pending.append(exchange('close'))
            events.append(token)

    error = ValueError(token)
    try:
        with closing(Resource()) as resource:
            assert resource is not None
            assert await exchange('body') == 'ok'
            raise error
    except ValueError as caught:
        assert caught is error
        assert events == [token]
        assert await pending.pop() == 'ok'
    else:
        assert False
    return json.dumps('ok')


asyncio.run(main())
