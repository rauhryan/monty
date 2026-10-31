"""Literal expectations and host scripts. Never import an adapter or coordinator here."""


def value(v):
    """Public outcome tuple, independent of the reference implementation."""
    return ['value', v, None, None, [], []]


def reply(n, op, v='ok', outcome='value', **extra):
    """Authored monotonically numbered host settlement."""
    return {
        'run': 'C1',
        'event': n,
        'kind': 'reply',
        'operation': 'O' + str(op),
        'outcome': outcome,
        'value': v,
        **extra,
    }


def cancel(n, reason):
    """Authored execution cancellation, including the empty first reason."""
    return {'run': 'C1', 'event': n, 'kind': 'cancel', 'reason': reason}


def request(n, *payload):
    """Literal ordered operation request."""
    return ['request', 'O' + str(n), list(payload)]


def settle(n, v='ok', kind='value'):
    """Literal acceptance (not necessarily guest delivery)."""
    return ['settle', 'O' + str(n), kind, v]


def case(mode, batches, events, trace, result):
    """Only authored data: the runner never updates these expectations."""
    return {'mode': mode, 'batches': batches, 'expected': {'events': events, 'trace': trace, 'result': result}}


CASES = {
    'nested': case(
        'nested',
        [[reply(1, 0, 'one')], [reply(2, 1, 'two')], [reply(3, 2)], [reply(4, 3)]],
        [
            request(0, 'first'),
            settle(0, 'one'),
            request(1, 'second'),
            settle(1, 'two'),
            request(2, 'close', 'inner'),
            settle(2),
            request(3, 'close', 'parent'),
            settle(3),
        ],
        ['one', 'two'],
        value('nested'),
    ),
    'queue': case(
        'queue',
        [[reply(1, 0)], [reply(2, 2, 'b')], [reply(3, 1, 'a')], [reply(4, 3)]],
        [
            request(0, 'gate'),
            settle(0),
            request(1, 'read', 'A'),
            request(2, 'read', 'B'),
            settle(2, 'b'),
            settle(1, 'a'),
            request(3, 'close', 'parent'),
            settle(3),
        ],
        ['put-A', 'put-B', ['read', 'B', 'b'], ['read', 'A', 'a']],
        value(['producer', 'A', 'B']),
    ),
    'host': case(
        'host',
        [[cancel(1, '')], [cancel(2, 'REPEAT'), reply(3, 1)]],
        [request(0, 'hold'), request(1, 'close', 'parent'), settle(1)],
        [['caught', '']],
        ['cancelled', 'suppressed-host', '', '', [], []],
    ),
    'handle': case(
        'handle',
        [[reply(1, 0)], [reply(2, 1)], [reply(3, 2)]],
        [
            request(0, 'close', 'child'),
            settle(0),
            request(1, 'retained'),
            settle(1),
            request(2, 'close', 'parent'),
            settle(2),
        ],
        ['child', 'delayed-ok', 'repeat-join-rejected', 'invalid-cancel'],
        value('handle'),
    ),
}
# Gate is O1: the child's operation is requested before the parent gate.
for name in ('queued', 'delivered', 'suppress', 'guest'):
    batch = [reply(1, 1), reply(2, 0, 'leaf-value')] if name == 'queued' else [reply(1, 0, 'leaf-value'), reply(2, 1)]
    if name in ('guest', 'suppress'):
        batch = [reply(1, 1)]
    events = [request(0, 'leaf'), request(1, 'gate')]
    events += [settle(int(m['operation'][1:]), m['value']) for m in batch]
    events += [request(2, 'close', 'child'), settle(2), request(3, 'close', 'parent'), settle(3)]
    trace = [['delivered', 'leaf-value']] if name == 'delivered' else [['caught', 'task-cancel:guest']]
    trace += (
        [['join', 'value', 'child', None]]
        if name == 'delivered'
        else (
            [['join', 'value', 'suppressed', None]]
            if name == 'suppress'
            else [['join', 'cancelled', None, 'task-cancel:guest']]
        )
    )
    CASES[name] = case(name, [batch, [reply(3, 2)], [reply(4, 3)]], events, trace, value('parent'))
CASES['mask'] = case(
    'mask',
    [[reply(1, 0)], [reply(2, 1)], [reply(3, 2)], [reply(4, 3)]],
    [
        request(0, 'gate'),
        request(1, 'close', 'inner'),
        settle(0),
        settle(1),
        request(2, 'close', 'child'),
        settle(2),
        request(3, 'close', 'parent'),
        settle(3),
    ],
    ['after-inner', ['join', 'value', 'child', None]],
    value('parent'),
)
CASES['drain'] = case(
    'drain',
    [[reply(1, 1)], [reply(2, 2, 'AUDIT', 'error')], [reply(3, 3, 'PARENT-AUDIT', 'error')], [reply(4, 4)]],
    [
        request(0, 'leaf'),
        request(1, 'gate'),
        settle(1),
        request(2, 'close', 'child'),
        settle(2, 'AUDIT', 'error'),
        request(3, 'close', 'parent-extra'),
        settle(3, 'PARENT-AUDIT', 'error'),
        request(4, 'close', 'parent'),
        settle(4),
    ],
    [['caught', 'owner-exit:S0']],
    [
        'error',
        None,
        'BODY',
        None,
        ['PARENT-AUDIT'],
        [['cancelled', None, 'owner-exit:S0', 'owner-exit:S0', ['AUDIT'], []]],
    ],
)
BAD = [
    reply(10, 0, run='OLD'),
    reply(0, 0, outcome='unsupported'),
    reply(0, 0),
    reply(2, 99, operation='missing'),
    reply(1, 0),
    reply(3, 0, 'valid'),
    reply(4, 0, 'stale'),
]
CASES['invalid'] = case(
    'invalid',
    [BAD, [reply(5, 1)]],
    [request(0, 'hold')]
    + [['reject', m] for m in BAD[:5]]
    + [settle(0, 'valid'), ['reject', BAD[6]], request(1, 'close', 'parent'), settle(1)],
    ['valid'],
    value('ok'),
)
MANDATORY = {
    'nested',
    'queue',
    'host',
    'handle',
    'queued',
    'delivered',
    'suppress',
    'guest',
    'mask',
    'drain',
    'invalid',
}
