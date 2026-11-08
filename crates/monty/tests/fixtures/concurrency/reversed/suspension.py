# ruff: noqa: F821
import asyncio
import json
from collections import deque


async def main():
    token = []
    token.append(token)
    items = [token, token]
    d = {'a': token, 'b': token}
    q = deque(items)
    li = reversed(items)
    di = reversed(d.items())
    ki = reversed(d)
    vi = reversed(d.values())
    qi = reversed(q)
    ri = reversed(range(10**12))
    wide = reversed(range(-(2**63), 2**63 - 1))
    minimum_step = reversed(range(2**63 - 1, -(2**63), -(2**63)))
    si = reversed('a🐍')
    assert await exchange('start') == 'ok'
    assert next(li) is token
    assert next(di) == ('b', token)
    assert next(ki) == 'b' and next(vi) is token
    assert next(qi) is token
    assert next(ri) == 10**12 - 1
    assert next(wide) == 2**63 - 2
    assert next(minimum_step) == -1
    assert next(si) == '🐍'
    assert await exchange('first') == 'ok'
    assert next(li) is token
    assert next(di) == ('a', token)
    assert next(ki) == 'a' and next(vi) is token
    assert next(qi) is token
    assert next(ri) == 10**12 - 2
    assert next(wide) == 2**63 - 3
    assert next(minimum_step) == 2**63 - 1
    assert next(si) == 'a'
    assert await exchange('second') == 'ok'
    for iterator in [li, di, ki, vi, qi, si]:
        assert next(iterator, None) is None
    assert await exchange('empty') == 'ok'
    di = reversed(d)
    qi = reversed(q)
    d['c'] = token
    q.append(token)
    for iterator in [di, qi]:
        try:
            next(iterator)
        except RuntimeError:
            pass
        else:
            assert False
    assert await exchange('mutated') == 'ok'
    try:
        next(di)
    except RuntimeError:
        pass
    else:
        assert False
    assert next(qi, None) is None
    return json.dumps('ok')


asyncio.run(main())
