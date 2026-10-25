# ruff: noqa: F821 -- assembled guest module with platform bindings and driver.
"""Guest-authored workflow. Bindings are supplied by the demo platform."""


async def add_to_cart(sku: str, quantity: int):
    async with timebox(5 * 60 * 1000) as scope:
        hold = await reserve(sku, quantity)
        scope.defer(release, hold)
        scope.defer(audit_release, hold)
        return await charge(hold)
