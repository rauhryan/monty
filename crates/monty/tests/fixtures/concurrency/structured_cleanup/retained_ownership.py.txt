"""Native live-heap fixtures for fresh scopes and rejected spawn input owners."""
# ruff: noqa: F821


async def parent_body(state, task, mode):
    async with Timebox(state, task, 50) as outer:
        outer.defer('outer-last', token)
        try:
            async with Timebox(state, task, 25) as inner:
                inner.defer('inner-last', token)
                await inner.operation(['gate'])
            await outer.operation(['after-inner'])
        except ValueError as error:
            assert str(error) == 'FIRST'
        async with Timebox(state, task, 10) as fresh:
            fresh.defer('fresh-last', token)
            if cut13 == 'new-scope':
                global checkpoint13
                checkpoint13 = state['events'][:]
                host()
            try:
                await fresh.spawn(token, token)
                assert False, 'cancelled ownership admitted a child'
            except ValueError as error:
                assert str(error) == 'scope-closed:S2'
            await fresh.operation(['fresh-body'])
        return 'returned'


def prepare13():
    """Accepted inner reply must survive deferred and repeated cancellation."""
    step11()
    settle11(value=token)
    step11()
    step11()
    settle11(value=token)
    cancel11('FIRST')
    cancel11('REPEAT-INNER')
    assert state['tasks']['T0']['resume'][1] is token
    step11()
    assert state['tasks']['T0']['resume'] == ['error', 'FIRST']
    step11()


def reject13():
    """Neither a host rejection nor a repeat may overwrite the pending error."""
    command(state, {'run': 'R11', 'kind': 'spawn', 'task': 'T0', 'scope': 'S2', 'input': token})
    cancel11('REPEAT-REJECTED')
    assert state['tasks']['T0']['resume'] == ['error', 'scope-closed:S2']
    assert state['next_task'] == 1
    assert state['next_boundary'] == 2
    assert state['scopes']['S2']['children'] == []
