# ruff: noqa: F401, F821, I001
from __monty_host__ import host_coroutines
import asyncio
import json

TRACE = []


def operation(name):
    return json.loads((yield exchange(json.dumps({'phase': name}))))


async def cleanup_exit_body(mode):
    token = []
    token.append(token)
    aliases = [token, token]
    try:
        if mode == 'error':
            raise TypeError('ORIGINAL')
        return aliases
    finally:
        try:
            TRACE.append('inner')
            await operation('inner')
        finally:
            assert aliases[0] is token and aliases[1] is token
            TRACE.append('outer')
            await operation('outer')


async def cleanup_exit_entry(mode):
    try:
        await cleanup_exit_body(mode)
        TRACE.append('FORBIDDEN')
    except asyncio.CancelledError as error:
        TRACE.append(['caught', str(error)])
    return None
