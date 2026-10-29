"""Additional literal reservation and scope-exit cases, fixed before their first run."""
from queue_gate import CASES
from proof3 import batch

CASES['reservation'] = ([batch([('put-reserved', 'ok')]), batch([('get-reserved', 'ok')])], 'kept')
CASES['scope_exit'] = (
    [batch([('exit', 'ok')]), batch([('cleanup-get', 'get-ok')]),
     batch([('cleanup-put', 'put-ok')]), batch([('parent-cleanup', 'ok')])],
    {'trace': [['put', 'scope-exit'], ['get', 'scope-exit'], ['get', 'get-ok'],
               ['put', 'put-ok'], 'parent-cleanup'], 'errors': []},
)
CASES['put_cancel'] = ([batch([('full', 'ok')])], {'values': ['seed', 'B'], 'puts': ['B']})
CASES['api'] = ([], ['value', 'value', 'value', 'type', 'type', 'accepted'])
