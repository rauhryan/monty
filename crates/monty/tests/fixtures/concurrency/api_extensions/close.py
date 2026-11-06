# ruff: noqa: F821
import asyncio
import json


def child(events, pending):
    try:
        yield 1
    except Exception:
        assert False, 'GeneratorExit must bypass Exception'
    finally:
        pending.append(exchange('close'))
        events.append('child')


def parent(events, pending):
    try:
        yield from child(events, pending)
    finally:
        events.append('parent')


def ignores_exit():
    try:
        yield 1
    except GeneratorExit:
        yield 2


def dormant(events):
    try:
        events.append('entered')
        yield 1
    finally:
        events.append('closed')


async def main():
    events = []
    pending = []
    gen = dormant(events)
    assert gen.close() is None
    assert events == []
    gen = parent(events, pending)
    assert next(gen) == 1
    assert gen.close() is None
    assert gen.close() is None
    assert events == ['child', 'parent']
    assert await pending[0] == 'ok'
    assert next(gen, 'end') == 'end'
    bad = ignores_exit()
    assert next(bad) == 1
    try:
        bad.close()
    except RuntimeError as error:
        assert str(error) == 'generator ignored GeneratorExit'
    else:
        assert False
    assert bad.close() is None
    return json.dumps('ok')


asyncio.run(main())
