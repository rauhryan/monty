"""Authored async functions; only supplied bindings expose cooperative requests."""
# ruff: noqa: F821


async def producer(state, task, mode):
    async with Scope(state, task, None) as scope:
        scope.defer('close', 'producer')
        for value in ['A', 'B', 'C', None, None]:
            await scope.put('work', value)
        return 'produced'


async def worker(state, task, mode):
    async with Scope(state, task, None) as scope:
        scope.defer('close', task['label'])
        scope.defer('audit', task['label'])
        results = []
        while True:
            value = await scope.get('work')
            if value is None:
                return results
            results.append(await scope.operation(['read', value]))


async def leaf(state, task, mode):
    async with Scope(state, task, None) as scope:
        scope.defer('close', task['label'])
        try:
            if mode == 'put':
                await scope.put('full', 'blocked')
            elif mode == 'get':
                await scope.get('empty')
            elif mode == 'join':
                child = await scope.spawn(leaf, 'grandchild', 'external')
                await scope.join(child)
            else:
                await scope.operation(['leaf', task['label']])
            return 'leaf-value'
        except ValueError as error:
            state['events'].append(['caught', task['label'], str(error)])
            if mode == 'suppress':
                return 'suppressed-task-cancel'
            raise


async def parent_body(state, task, mode):
    state['queues'] = {
        'work': {'items': [], 'put': [], 'get': []},
        'empty': {'items': [], 'put': [], 'get': []},
        'full': {'items': ['occupied'], 'put': [], 'get': []},
    }
    try:
        async with Scope(state, task, None) as scope:
            scope.defer('close', 'parent')
            if mode in (
                'success',
                'body-error',
                'host-error',
                'host-cancel',
                'host-cancel-cleanup-error',
                'host-wake-cancel',
                'cleanup-error',
                'early',
            ):
                p = await scope.spawn(producer, 'producer')
                a = await scope.spawn(worker, 'worker-a')
                b = await scope.spawn(worker, 'worker-b')
                if mode == 'body-error':
                    raise ValueError('body-original')
                if mode == 'early':
                    return 'early'
                outcomes = []
                for child in [p, a, b]:
                    outcome = await scope.join(child)
                    outcomes.append(outcome)
                    if outcome['kind'] == 'error':
                        raise ValueError('joined-child-error')
                try:
                    await scope.join(a)
                except ValueError as error:
                    state['events'].append(['join-error', str(error)])
                return outcomes
            async with Scope(state, task, None) as inner:
                inner.defer('close', 'inner')
                child = await inner.spawn(leaf, 'victim', mode)
                # A fixed cooperative gate lets the child park, including its
                # nested owned child. It is not a host task-selection command.
                for _ in range(4):
                    await inner.checkpoint()
                await inner.operation(['cancel-gate'])
                await inner.cancel(child, 'explicit')
                outcome = await inner.join(child)
                state['events'].append(['joined', outcome])
                return outcome
    except ValueError as error:
        state['events'].append(['parent-caught', str(error)])
        if mode in ('host-cancel', 'host-wake-cancel', 'host-cancel-cleanup-error'):
            async with Scope(state, task, None) as fresh:
                try:
                    await fresh.spawn(leaf, 'forbidden', 'external')
                except ValueError as closed:
                    state['events'].append(['admission-closed', str(closed)])
            return 'caught-host'
        raise
