import asyncio
import json


async def cancelled(signal):
    raise signal


async def caught():
    try:
        raise asyncio.CancelledError('caught')
    except asyncio.CancelledError:
        return 'success'


async def replaced():
    try:
        raise asyncio.CancelledError('replaced')
    finally:
        raise ValueError('cleanup')


async def main():
    token = []
    signal = asyncio.CancelledError(token)
    token.append(signal)
    task = asyncio.create_task(cancelled(signal))
    await exchange('before')
    try:
        await task
    except asyncio.CancelledError as error:
        assert error is signal
    else:
        assert False, 'expected cancellation'
    await exchange('completed')
    assert task.done()
    assert task.cancelled()
    for inspect in [task.result, task.exception]:
        try:
            inspect()
        except asyncio.CancelledError:
            pass
        else:
            assert False, 'expected cancellation inspection'
    success = asyncio.create_task(caught())
    assert await success == 'success'
    assert not success.cancelled()
    failure = asyncio.create_task(replaced())
    try:
        await failure
    except ValueError as error:
        assert str(error) == 'cleanup'
        assert failure.exception() is error
    else:
        assert False, 'expected cleanup error'
    assert not failure.cancelled()
    return json.dumps('ok')


asyncio.run(main())
