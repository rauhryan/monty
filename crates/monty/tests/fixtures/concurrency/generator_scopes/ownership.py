"""Native counted-test helpers over the actual child-scope library (not JSON reconstruction)."""

# Assembled after scope7_guest.py by the native counted tests.
# ruff: noqa: F821


def step9(task='T0'):
    """Choose one task; never auto-drain runnable work."""
    command(state, {'run': 'R7', 'kind': 'advance', 'task': task})


def settle9(task, value=None, outcome='value'):
    """Queue an owning value or error at the selected operation."""
    wait = state['tasks'][task]['wait']
    command(
        state,
        {'run': 'R7', 'kind': 'settle', 'task': task, 'boundary': wait['boundary'], 'outcome': outcome, 'value': value},
    )


def cancel9(task='T0'):
    """Latch scope intent independently of an operation reply."""
    command(
        state,
        {
            'run': 'R7',
            'kind': 'cancel',
            'task': task,
            'scope': state['tasks'][task]['scope'],
            'reason': 'counted-cancel',
        },
    )


def start9():
    """Park both child frames with shared handle aliases."""
    step9()
    step9()
    step9()
    step9('T1')
    settle9('T1', token)
    step9('T1')
    step9('T2')
    settle9('T2', token)
    step9('T2')


def finish9(task):
    """Close a body, yield and settle its deferred release, then exit."""
    step9(task)
    step9(task)
    settle9(task)
    step9(task)
