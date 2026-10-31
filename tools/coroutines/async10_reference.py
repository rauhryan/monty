"""Independent generator spelling of the bounded syntax example, without async with."""


def cart(events, failures, mode):
    events.append(['enter', 50])
    yield ['scope-enter', 50]
    cause = None
    hold = None
    acquired = False
    try:
        hold = yield ['acquire']
        acquired = True
        if mode == 'return':
            return hold
        if mode in ('error', 'suppress'):
            raise ValueError('body-failed')
        try:
            return (yield ['use', hold])
        except ValueError:
            if mode == 'catch':
                return 'caught'
            raise
    except Exception as error:
        cause = str(error)
        if mode != 'suppress':
            raise
    finally:
        events.append(['exit', cause])
        if acquired:
            for operation in ('audit-release', 'release'):
                try:
                    yield [operation, hold]
                except Exception as error:
                    failures.append(str(error))
    return 'suppressed'
