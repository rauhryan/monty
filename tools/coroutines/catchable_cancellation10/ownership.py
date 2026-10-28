# ruff: noqa: F401, F821
from __monty_host__ import host_coroutines

TRACE = []


def operation(name):
    return (yield exchange(name))


async def repair10(mode):
    token = []
    token.append(token)
    original = TypeError(token)
    alias = original
    try:
        try:
            raise original
        finally:
            assert original is alias
            await operation('cleanup')
            assert original is alias
    except ValueError as signal:
        assert str(signal) == 'FIRST'
        TRACE.append('caught')
        if mode == 'uncaught':
            raise
        await operation('recovery')
        nested = []
        for i in range(32):
            nested = [nested]
        result = [[42], token, nested]
        if mode == 'later':
            try:
                return result
            finally:
                raise TypeError('LATER')
        return result
