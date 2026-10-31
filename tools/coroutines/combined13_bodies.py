"""R12-1 literal test body and graph projection; assembled with the real library."""

ASYNC = """
async def parent_body(state, task, mode):
    async with Timebox(state, task, 50) as outer:
        outer.defer('outer-last', 'P')
        try:
            if mode != 'immediate':
                async with Timebox(state, task, 25) as inner:
                    inner.defer('inner-last', 'I')
                    await inner.operation(['gate'])
            await outer.operation(['after-inner'])
        except ValueError as error:
            state['events'].append(['caught', str(error)])
        async with Timebox(state, task, 10) as fresh:
            fresh.defer('fresh-last', 'F')
            for label in ['fresh', 'again']:
                try:
                    child = await fresh.spawn(label, 'normal')
                    state['events'].append(['new-child', child])
                except ValueError as error:
                    state['events'].append(['caught-spawn', str(error)])
            await fresh.operation(['fresh-body'])
        return 'returned'
"""

REVIEW = """
async def parent_body(state, task, mode):
    async with Timebox(state, task, 50) as outer:
        outer.defer('outer-last', 'P')
        try:
            async with Timebox(state, task, 25) as inner:
                inner.defer('inner-last', 'I')
                await inner.operation(['gate'])
            await outer.operation(['after-inner'])
        except ValueError as error:
            state['events'].append(['caught', str(error)])
        async with Timebox(state, task, 10) as fresh:
            fresh.defer('fresh-last', 'F')
            child = await fresh.spawn('fresh', 'normal')
            state['events'].append(['new-child', child])
            await fresh.operation(['fresh-body'])
        return 'returned'
"""

# Include admission inputs in every backend observation, rather than trusting
# CPython's shared-driver equality as an independent policy oracle.
PROJECTION = """
original_view13 = view
def view(state):
    result = original_view13(state)
    result['admission'] = {
        'next_task': state['next_task'],
        'next_scope': state['next_scope'],
        'next_boundary': state['next_boundary'],
        'tasks': [[t['id'], t['owner'], t['phase'], t['status'], t['resume'], t['cancel'],
                   t['cancel_pending'], t['stack'], t['pending_join']]
                  for t in state['tasks'].values()],
        'scopes': [[s['id'], s['task'], s['parent'], s['phase'], s['children'], s['cursor'], s['failures']]
                   for s in state['scopes'].values()],
    }
    return result
"""
