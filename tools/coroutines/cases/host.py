import json

exchange: object  # Supplied by the host or the CPython reference driver.

events = ['prefix']
shared = []
alias = shared


def work():
    """Keep an error pending across a second external call in finally."""
    try:
        answer = json.loads(exchange(json.dumps({'at': 'work', 'events': events})))
        shared.append(answer['send'])
    finally:
        events.append('cleanup')
        exchange(json.dumps({'at': 'cleanup', 'events': events}))
        assert alias is shared
        events.append('cleaned')


try:
    work()
    result = {'value': shared, 'events': events}
except ValueError as error:
    result = {'error': ['ValueError', str(error)], 'events': events}
json.dumps(result)
