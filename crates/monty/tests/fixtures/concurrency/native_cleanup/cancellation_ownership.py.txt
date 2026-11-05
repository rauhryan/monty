from __monty_host__ import host_coroutines
import asyncio
import json

TRACE = []


def operation(name):
    return json.loads((yield exchange(json.dumps({'phase': name}))))


async def cancellation_body():
    token = []
    token.append(token)
    aliases = [token, token]
    try:
        try:
            await operation('body')
        except asyncio.CancelledError as error:
            TRACE.append(['caught', str(error)])
            return 'caught'
        finally:
            TRACE.append('inner')
            await operation('inner')
    finally:
        assert aliases[0] is token and aliases[1] is token
        TRACE.append('outer')
        await operation('outer')
