import json

exchange: object  # Supplied by the host or the CPython reference driver.


def factory():
    """Share a rebound closure cell and a cyclic object across delegation."""
    count = 0
    shared = []
    shared.append(shared)

    def leaf():
        """Accept a reply or failure, yielding cleanup while failure is pending."""
        nonlocal count
        count += 1
        try:
            reply = yield {'at': 'leaf', 'count': count}
            if reply == 'fail':
                raise ValueError('guest failure')
            count += reply
        finally:
            ack = yield {'at': 'cleanup', 'count': count}
            assert ack == 'clean'
            count += 10
        return shared

    def read():
        """Observe the same cell without exporting addresses."""
        assert shared[0] is shared
        return count

    return leaf, read, shared


def delegate(child):
    """Forward send, throw, and the exact StopIteration return object."""
    return (yield from child)


leaf, read, shared = factory()
g = delegate(delegate(leaf()))
alias = g
effect = next(g)
while True:
    command = json.loads(exchange(json.dumps(effect)))
    assert alias is g
    try:
        if 'throw' in command:
            effect = alias.throw(ValueError(command['throw']))
        else:
            effect = alias.send(command['send'])
    except StopIteration as finished:
        assert finished.value is shared
        result = {'value_identity': True, 'count': read()}
        break
    except ValueError as error:
        result = {'error': ['ValueError', str(error)], 'count': read()}
        break
json.dumps(result)
