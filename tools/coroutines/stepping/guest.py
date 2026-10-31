"""Authored guest: reserve, charge, and release even when charging fails."""


async def checkout():
    """Contracts are supplied by the embedding environment."""
    hold = await reserve('SKU-123', 2)
    try:
        return await charge(hold['total'])
    finally:
        await release(hold['id'])
