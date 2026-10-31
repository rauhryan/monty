from __monty_host__ import host_coroutines
import json


async def parent():
    outcomes = []
    for capacity in [0, -1, 4294967296, True, 1.5, 4294967295]:
        try:
            asyncio._queue(capacity)
            outcomes.append('accepted')
        except ValueError:
            outcomes.append('value')
        except TypeError:
            outcomes.append('type')
    return json.dumps(outcomes)


parent()._native1()
