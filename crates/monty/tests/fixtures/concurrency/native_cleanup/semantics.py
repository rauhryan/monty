from __monty_host__ import host_coroutines
import json

TRACE = []


def native_operation(label):
    return json.loads((yield exchange(json.dumps({'phase': label, 'trace': TRACE}))))


class Resource:
    def __init__(self, token):
        self.a = token
        self.b = token

    async def __aenter__(self):
        assert self.a is self.b
        return self

    async def __aexit__(self, typ, error, tb):
        assert self.a is self.b and self.a[0] is self.a
        try:
            TRACE.append(['audit', await native_operation('audit')])
        except ValueError as audit_error:
            TRACE.append(['audit-error', str(audit_error)])
        TRACE.append(['close', await native_operation('close')])
        return False


async def semantic_body():
    token = []
    token.append(token)
    async with Resource(token):
        try:
            value = await native_operation('read')
            TRACE.append(['read', value])
            return value
        except ValueError as error:
            TRACE.append(['caught', str(error)])
            return 'caught'
