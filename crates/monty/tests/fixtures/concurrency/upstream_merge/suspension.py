# ruff: noqa: F821
import annotationlib
import asyncio
import json
import typing


def annotated(value: int) -> str:
    return str(value)


def broken(cause):
    if False:
        yield
    raise cause


async def child(cause):
    try:
        raise cause
    except ValueError:
        await exchange('handled')
        raise RuntimeError('replacement') from cause


async def main():
    assert typing.get_type_hints(annotated) == {'value': int, 'return': str}

    class Owner:
        value: int

    forward = annotationlib.ForwardRef('int', owner=Owner)
    cause = ValueError('original')
    task = asyncio.create_task(child(cause))
    try:
        await task
    except RuntimeError as error:
        assert error.__cause__ is cause
        assert error.__context__ is cause
        assert error.__suppress_context__ is True
        await exchange('chained')
        assert forward.__owner__ is Owner
        assert forward.evaluate() is int
        assert error.__cause__ is cause
    token = []
    stop = StopIteration(token)
    iterator = broken(stop)
    try:
        next(iterator)
    except RuntimeError as error:
        assert error.__cause__ is stop
        assert error.__context__ is stop
        assert error.__suppress_context__ is True
        await exchange('generator')
        assert error.__cause__.value is token
    return json.dumps('ok')


asyncio.run(main())
