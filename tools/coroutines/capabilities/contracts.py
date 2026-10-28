from __monty_host__ import host_coroutines  # noqa: F401, I001
import json

# Supplied embedding names and native exchange are resolved by Monty, not CPython.
# ruff: noqa: F821

LOCAL_LABEL = 0


def effect(cap, op, arguments):
    """Private binding; local labels route replies, never authorize effects."""
    global LOCAL_LABEL
    label = LOCAL_LABEL
    LOCAL_LABEL += 1
    request = {'id': label, 'cap': cap, 'op': op, 'args': arguments}
    return json.loads((yield exchange(json.dumps(request))))['value']


async def reserve(quantity):
    """Authored inventory contract, scoped by a host-granted handle."""
    return await effect(INVENTORY, 'reserve', {'resource': 'SKU-123', 'quantity': quantity})


async def charge(amount):
    """Authored payment contract for one account with a per-effect amount limit."""
    return await effect(PAYMENT, 'charge', {'resource': 'account-1', 'amount': amount})


async def release():
    """Pregranted release authority for hold-1 only, independent of payment authority."""
    return await effect(RELEASE, 'release', {'resource': 'hold-1'})
