# ruff: noqa: F821 -- assembled guest module with platform bindings and driver.
"""Platform-supplied guest bindings, assembled before the authored workflow."""


def _effect(contract, arguments):
    return (yield [contract, arguments])


async def reserve(sku: str, quantity: int):
    return await _effect('reserve', {'sku': sku, 'quantity': quantity})


async def charge(hold: str):
    return await _effect('charge', {'hold': hold})


async def release(hold: str):
    return await _effect('release', {'hold': hold})


async def audit_release(hold: str):
    return await _effect('audit-release', {'hold': hold})


class Scope:
    def __init__(self, duration_ms):
        self.duration_ms = duration_ms
        self.deferred = []

    async def __aenter__(self):
        _events.append(['enter', self.duration_ms])
        await _effect('scope-enter', {'duration_ms': self.duration_ms})
        return self

    def defer(self, operation, *args):
        self.deferred.append((operation, args))

    async def __aexit__(self, typ, value, traceback):
        _events.append(['exit', None if typ is None else str(value)])
        while self.deferred:
            operation, args = self.deferred.pop()
            try:
                await operation(*args)
            except Exception as error:
                _failures.append(str(error))
        return False


def timebox(duration_ms):
    return Scope(duration_ms)


async def cart(events, failures, mode):
    # Adapter to the retained sequential driver. One task per demo run.
    global _events, _failures
    _events = events
    _failures = failures
    return await add_to_cart('coffee', 2)
