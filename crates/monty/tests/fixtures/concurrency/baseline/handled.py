# ruff: noqa: F821
import asyncio
import json


async def main():
    token = []
    token.append(token)
    error = ValueError(token)
    try:
        raise error
    except ValueError as caught:
        assert caught is error
        assert await exchange('handled') == 'ok'
        assert caught.args[0] is token and token[0] is token
    return json.dumps('ok')


asyncio.run(main())
