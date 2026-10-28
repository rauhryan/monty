"""Authored programs: TRACE is application observation, not scheduler diagnostics."""

# ruff: noqa: F821
TRACE = []


async def leaf(state, task, mode):
    async with Scope(state, task, None) as scope:
        scope.defer('close', 'child')
        if mode == 'done':
            return 'child'
        if mode == 'mask':
            async with Scope(state, task, None) as inner:
                inner.defer('close', 'inner')
            TRACE.append('after-inner')
            return 'child'
        try:
            value = await scope.operation(['leaf'])
            TRACE.append(['delivered', value])
            return 'child'
        except ValueError as error:
            TRACE.append(['caught', str(error)])
            if mode == 'suppress':
                return 'suppressed'
            raise


async def producer(state, task, mode):
    async with Scope(state, task, None) as scope:
        await scope.put('q', 'A')
        TRACE.append('put-A')
        await scope.put('q', 'B')
        TRACE.append('put-B')
        return 'producer'


async def consumer(state, task, mode):
    async with Scope(state, task, None) as scope:
        item = await scope.get('q')
        value = await scope.operation(['read', item])
        TRACE.append(['read', item, value])
        return item


async def parent_body(state, task, mode):
    async with Scope(state, task, None) as scope:
        scope.defer('close', 'parent')
        if mode == 'drain':
            scope.defer('close', 'parent-extra')
        if mode == 'nested':
            TRACE.append(await scope.operation(['first']))
            async with Scope(state, task, None) as inner:
                inner.defer('close', 'inner')
                TRACE.append(await inner.operation(['second']))
            return 'nested'
        if mode == 'queue':
            state['queues']['q'] = {'items': [], 'get': [], 'put': []}
            p = await scope.spawn(producer, 'producer')
            # An explicit gate proves the producer is backpressured before consumers exist.
            await scope.operation(['gate'])
            a = await scope.spawn(consumer, 'a')
            b = await scope.spawn(consumer, 'b')
            outcomes = [await scope.join(p), await scope.join(a), await scope.join(b)]
            return [r['value'] for r in outcomes]
        if mode in ('host', 'invalid'):
            try:
                TRACE.append(await scope.operation(['hold']))
            except ValueError as error:
                TRACE.append(['caught', str(error)])
                return 'suppressed-host'
            return 'ok'
        child = await scope.spawn(leaf, 'child', 'done' if mode == 'handle' else mode)
        if mode == 'handle':
            result = await scope.join(child)
            TRACE.append(result['value'])
            for _ in range(80):
                await scope.checkpoint()
            await scope.operation(['retained'])
            await scope.cancel(child, 'delayed')
            TRACE.append('delayed-ok')
            try:
                await scope.join(child)
            except ValueError:
                TRACE.append('repeat-join-rejected')
            else:
                raise ValueError('repeat-join-accepted')
            async with Scope(state, task, None) as other:
                try:
                    await other.cancel(child, 'foreign')
                except ValueError as error:
                    TRACE.append(str(error))
                else:
                    raise ValueError('foreign-cancel-accepted')
            return 'handle'
        await scope.operation(['gate'])
        if mode == 'drain':
            raise ValueError('BODY')
        await scope.cancel(child, 'guest')
        result = await scope.join(child)
        TRACE.append(['join', result['kind'], result['value'], result['cause']])
        return 'parent'
