"""Helpers for native before-teardown ownership checks of the actual library."""
# ruff: noqa: F821


def step11(task='T0'):
    command(state, {'run': 'R11', 'kind': 'advance', 'task': task})


def settle11(task='T0', value=None, outcome='value'):
    command(
        state,
        {
            'run': 'R11',
            'kind': 'settle',
            'task': task,
            'boundary': state['tasks'][task]['wait']['boundary'],
            'outcome': outcome,
            'value': value,
        },
    )


def cancel11(reason='alarm'):
    command(state, {'run': 'R11', 'kind': 'cancel', 'task': 'T0', 'scope': 'S0', 'reason': reason})


def start11():
    step11()
    step11()
    step11()
    for task in ['T1', 'T2']:
        step11(task)
        settle11(task, token)
        step11(task)


def retire11(task):
    step11(task)
    step11(task)
    settle11(task)
    step11(task)
    settle11(task)
    step11(task)
