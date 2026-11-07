# ruff: noqa: F821
import asyncio
import anyio
import json


class Iterator:
    def __iter__(self):
        return self

    def __next__(self):
        return exchange('body')

    def close(self):
        assert False, 'cancellation must not close the iterator'


class Cancelling:
    def __init__(self, scope, signals):
        self.scope = scope
        self.signals = signals

    def __iter__(self):
        return self

    def __next__(self):
        self.scope.cancel()
        try:
            exchange('forbidden')
        except asyncio.CancelledError as signal:
            self.signals.append(signal)
            raise
        assert False


async def main():
    signals = []
    with anyio.CancelScope() as scope:
        for item in Cancelling(scope, signals):
            assert False
    assert scope.cancelled_caught and len(signals) == 1
    try:
        for pending in Iterator():
            await pending
            assert False
    except asyncio.CancelledError as signal:
        assert str(signal) == 'FIRST'
        with anyio.CancelScope(shield=True):
            await exchange('cleanup')
        return json.dumps('suppressed')
    assert False


asyncio.run(main())
