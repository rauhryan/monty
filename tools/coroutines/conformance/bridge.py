"""Baseline-only translation: private Python data never enters the adapter contract."""

# ruff: noqa: F821

IDS = []
PUBLIC = []


def outcome(result):
    """Expose semantic outcomes without the task record representation."""
    if result is None:
        return None
    return [
        result['kind'],
        result['value'],
        result['cause'],
        result['cancel'],
        result['failures'],
        [outcome(child) for child in result['children']],
    ]


def op_id(raw):
    """First request assigns identity; subsequent routing must preserve that identity."""
    return 'O' + str(IDS.index(raw)) if raw in IDS else raw


def collect(events):
    """Preserve all ordered public requests, acceptances and rejections."""
    for event in events:
        if event[0] == 'pause':
            raw = event[1]['boundary']
            assert raw not in IDS
            IDS.append(raw)
            PUBLIC.append(['request', op_id(raw), event[1]['payload']])
        elif event[0] == 'accept':
            PUBLIC.append(['settle', op_id(event[1]), event[2], event[3]])
        elif event[0] == 'reject':
            message = dict(event[1])
            if 'operation' in message:
                message['operation'] = op_id(message['operation'])
            PUBLIC.append(['reject', message])


def step(messages):
    """Drive to externally blocked/complete; instruction budgets are not semantic cuts."""
    translated = []
    for original in messages:
        message = dict(original)
        if 'operation' in message and message['operation'].startswith('O'):
            message['operation'] = IDS[int(message['operation'][1:])]
        translated.append(message)
    for index in range(2000):
        previous = state['events'][-1] if state['events'] else None
        turn(state, translated if index == 0 else [], 1)
        events = state['events']
        start = 0
        if previous is not None:
            for offset in range(len(events)):
                if events[offset] is previous:
                    start = offset + 1
                    break
            else:
                raise ValueError('observation-overflow')
        collect(events[start:])
        if not state['ready']:
            return {'events': PUBLIC, 'trace': TRACE, 'result': outcome(state['tasks']['T0']['result'])}
    raise ValueError('non-quiescent')
