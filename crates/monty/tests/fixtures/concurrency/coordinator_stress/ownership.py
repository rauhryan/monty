"""Native memory-model fixture: all temporary traversal owners are function locals."""
# ruff: noqa: F821


def settle_stress():
    messages = []
    serial = state['event_watermark']
    for task in state['tasks'].values():
        if task['wait'] is not None:
            serial += 1
            messages.append(
                {
                    'run': 'C1',
                    'event': serial,
                    'kind': 'reply',
                    'operation': task['wait']['boundary'],
                    'outcome': 'value',
                    'value': None,
                }
            )
    pump(state, messages)


def cancel_stress():
    pump(state, [{'run': 'C1', 'event': state['event_watermark'] + 1, 'kind': 'cancel', 'reason': ''}])
