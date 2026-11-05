"""Async spellings of accepted scope7 bodies; driver ownership policy is unchanged."""


def request(descriptor):
    return (yield descriptor)


async def parent_body(task, mode):
    try:
        return await parent_work(task, mode)
    except ValueError:
        if mode == 'catch':
            return 'caught-parent-success'
        raise


async def parent_work(task, mode):
    task['deferred'].append(['release', 'parent-H'])
    first = await request({'kind': 'spawn', 'label': 'a', 'mode': mode})
    second = await request({'kind': 'spawn', 'label': 'b', 'mode': mode})
    if mode == 'early':
        return 'early'
    if mode == 'early-live':
        await request({'kind': 'operation', 'payload': ['parent-work']})
        return 'early-live'
    one = await request({'kind': 'join', 'child': first})
    if one['kind'] == 'error' and mode != 'ignore-error':
        cause = one['cause']
        raise ValueError('child:failed' if cause is None else 'child:' + cause)
    two = await request({'kind': 'join', 'child': second})
    return [one, two]


async def child_body(task, mode):
    hold = await request({'kind': 'operation', 'payload': ['acquire', task['label']]})
    task['deferred'].append(['release', hold])
    if mode == 'multi':
        task['deferred'].append(['audit-release', hold])
    try:
        return await request({'kind': 'operation', 'payload': ['use', hold]})
    except ValueError:
        if mode == 'catch':
            return 'caught-success'
        raise
