"""Authored async checkout; no host journal or scheduler implementation lives here."""
# Contracts and AFTER_CLEANUP are supplied by the embedding.
# ruff: noqa: F821


async def checkout():
    """Release the fixed reservation even on cancellation or a declined payment."""
    hold = await reserve(2)
    try:
        receipt = await charge(hold['total'])
    finally:
        await release()
    if AFTER_CLEANUP:
        # Must remain unreachable until the VM delivers pending cancellation.
        await charge(1)
    return receipt


async def entry():
    """Keep ordinary host denials distinct from native sticky cancellation."""
    try:
        return {'status': 'succeeded', 'value': await checkout()}
    except ValueError as error:
        return {'status': 'failed', 'error': str(error)}
