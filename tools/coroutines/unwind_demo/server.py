"""Local execution inspector backed by fresh Monty Wasm instances."""

import argparse
import hashlib
import json
import subprocess
import tempfile
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path

ROOT = Path(__file__).resolve().parent
COROUTINES = ROOT.parent
REPO = COROUTINES.parents[1]

# Host-owned allowlist and input contracts for this fixed demonstration.
CONTRACTS = {
    'scope-enter': {'duration_ms': int},
    'reserve': {'sku': str, 'quantity': int},
    'charge': {'hold': str},
    'release': {'hold': str},
    'audit-release': {'hold': str},
}


class Demo:
    def __init__(self, directory):
        self.directory = Path(directory)
        self.snapshot = self.directory / 'snapshot.bin'
        self.source = self.directory / 'guest.py'
        self.guest = (ROOT / 'cart.py').read_text()
        self.bindings = (ROOT / 'bindings.py').read_text()
        self.source.write_text(
            'from __monty_host__ import host_coroutines\n'
            + self.bindings
            + '\n'
            + self.guest
            + '\ndef adapt(body):\n    return body\n'
            + (COROUTINES / 'async10_driver.py').read_text()
            + "\nimport json\nstate = new_state('normal')\n"
            + "while state['terminal'] is None:\n"
            + '    message = json.loads(exchange(json.dumps(observe(state))))\n'
            + '    apply(state, message)\n'
            + 'json.dumps(observe(state))\n'
        )
        self.state = None
        self.trace = []
        self.revision = 0
        self.disposed = False
        self.activations = 0

    def invoke(self, *args):
        result = subprocess.run(
            ['node', str(REPO / 'playground/coroutine-baseline/wasm.mjs'), *map(str, args)],
            capture_output=True,
            text=True,
            timeout=30,
            check=True,
        )
        self.activations += 1
        return json.loads(result.stdout)

    def step(self, message=None):
        output = self.directory / 'next.bin'
        if message is None:
            event = self.invoke('start', self.source, output)
        else:
            event = self.invoke('resume', self.snapshot, json.dumps(message), output)
        if 'effect' in event:
            output.replace(self.snapshot)
            self.state = event['effect']
            pending = self.state['pending']
            if pending is not None:
                name, arguments = pending
                schema = CONTRACTS.get(name)
                if schema is None or not isinstance(arguments, dict) or arguments.keys() != schema.keys():
                    raise ValueError('Unknown contract or invalid arguments')
                if any(type(arguments[key]) is not typ for key, typ in schema.items()):
                    raise ValueError('Contract argument type mismatch')
        elif 'complete' in event:
            self.snapshot.unlink(missing_ok=True)
            self.state = event['complete']
        else:
            raise ValueError(f'Unexpected runtime result: {event}')
        self.trace.append({'command': message or {'kind': 'start'}, 'state': self.state})
        self.trace = self.trace[-100:]

    def command(self, action, revision):
        if revision != self.revision:
            raise ValueError('State changed. Reload before continuing.')
        if action not in ('start', 'complete', 'fail', 'cancel', 'dispose', 'restore'):
            raise ValueError('Unknown command')
        if action == 'start':
            if self.state is not None and not self.disposed and self.state['terminal'] is None:
                raise ValueError('Finish or explicitly discard the current run first')
            self.trace = []
            self.disposed = False
            self.activations = 0
            self.step()
            self.step({'kind': 'advance'})
            # Scope registration and reservation are local demo host replies.
            self.settle(None)
            self.settle('cart-hold-1')
        else:
            if self.state is None or self.disposed or self.state['terminal'] is not None:
                raise ValueError('Start a run first')
            if action in ('restore', 'dispose'):
                # Load and physically discard an independent instance without a reply.
                assert self.invoke('dispose', self.snapshot) == {'disposed': True}
                self.trace.append({'command': {'kind': action}, 'state': self.state})
                if action == 'dispose':
                    self.snapshot.unlink()
                    self.disposed = True
            elif action == 'cancel':
                self.step({'kind': 'cancel', 'value': 'Cancelled by host'})
                if self.state['status'] == 'runnable':
                    self.step({'kind': 'advance'})
            else:
                self.settle('cleanup failed' if action == 'fail' else 'ok', action == 'fail')
        self.revision += 1
        return self.view()

    def settle(self, value, failed=False):
        self.step({'kind': 'settle', 'outcome': 'error' if failed else 'value', 'value': value})
        self.step({'kind': 'advance'})

    def view(self):
        data = self.snapshot.read_bytes() if self.snapshot.exists() else b''
        return {
            'revision': self.revision,
            'state': self.state,
            'disposed': self.disposed,
            'trace': self.trace,
            'source': self.guest,
            'bindings': self.bindings,
            'activations': self.activations,
            'snapshot': {'bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest() if data else None},
        }


def serve(port):
    with tempfile.TemporaryDirectory(prefix='monty-unwind-') as directory:
        demo = Demo(directory)

        class Handler(BaseHTTPRequestHandler):
            def send(self, status, body, content_type='application/json'):
                self.send_response(status)
                self.send_header('Content-Type', content_type)
                self.send_header('Cache-Control', 'no-store')
                self.send_header('X-Content-Type-Options', 'nosniff')
                self.end_headers()
                self.wfile.write(body)

            def do_GET(self):
                if self.path == '/api':
                    self.send(200, json.dumps(demo.view()).encode())
                elif self.path == '/':
                    self.send(200, (ROOT / 'index.html').read_bytes(), 'text/html; charset=utf-8')
                else:
                    self.send(404, b'{}')

            def do_POST(self):
                if self.path != '/api' or self.headers.get('Content-Type') != 'application/json':
                    self.send(400, b'{}')
                    return
                origin = self.headers.get('Origin')
                if origin and origin != f'http://{self.headers.get("Host")}':
                    self.send(403, b'{}')
                    return
                try:
                    size = int(self.headers.get('Content-Length', '0'))
                    if not 0 < size < 1024:
                        raise ValueError('Invalid request size')
                    request = json.loads(self.rfile.read(size))
                    result = demo.command(request['action'], request['revision'])
                    self.send(200, json.dumps(result).encode())
                except (ValueError, KeyError, subprocess.SubprocessError, OSError) as error:
                    self.send(400, json.dumps({'error': str(error)}).encode())

        server = HTTPServer(('127.0.0.1', port), Handler)
        print(f'Monty unwind demo: http://127.0.0.1:{server.server_port}', flush=True)
        try:
            server.serve_forever()
        finally:
            server.server_close()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--port', type=int, default=4318)
    serve(parser.parse_args().port)
