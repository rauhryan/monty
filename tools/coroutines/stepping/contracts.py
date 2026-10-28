from __monty_host__ import host_coroutines
import json

# Supplied guest bindings, compiled with authored code in this experiment.
# No queue, task selection, clock, or host I/O is implemented here.
NEXT_REQUEST = 0


def effect(name, arguments):
    """Private continuation binding; authors use the async contracts below."""
    global NEXT_REQUEST
    request_id = NEXT_REQUEST
    NEXT_REQUEST += 1
    descriptor = {'id': request_id, 'name': name, 'arguments': arguments}
    reply = json.loads((yield exchange(json.dumps(descriptor))))
    return reply['value']


async def reserve(sku, quantity):
    """Request a reservation from the host."""
    return await effect('reserve', {'sku': sku, 'quantity': quantity})


async def charge(total):
    """Request a charge from the host."""
    return await effect('charge', {'total': total})


async def release(hold_id):
    """Request release; this can suspend during finally."""
    return await effect('release', {'hold_id': hold_id})


async def entry():
    """Export this example's value/error outcome after authored cleanup."""
    try:
        return {'status': 'succeeded', 'value': await checkout()}
    except ValueError as error:
        return {'status': 'failed', 'error': str(error)}
