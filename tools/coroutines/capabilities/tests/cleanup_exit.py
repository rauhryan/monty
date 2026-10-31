"""Cancellation delivery precedes authored recovery; host authority still denies its charge."""

import argparse
import json
from pathlib import Path

from demo import provision
from host import Finished
from journal import Denied
from repair import record, start

BODY = """
TRACE = []
async def entry():
    try:
        try:
            pass
        finally:
            await release()
        TRACE.append('FORBIDDEN')
    except ValueError as error:
        TRACE.append(['delivered', str(error)])
    try:
        await charge(1)
    except ValueError as error:
        TRACE.append(['denied', str(error)])
    return TRACE
"""


def main():
    """Real fresh-process restore, retained waits and canonical replay on both backends."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    results = {}
    for backend in ('native', 'wasm'):
        directory = args.output / backend
        directory.mkdir()
        host, handles = provision(directory)
        try:
            cleanup = start(directory, host, handles, backend, BODY)
            first = host.advance(cleanup, cancel='')
            repeat = host.advance(first, cancel='REPEAT')
            purchase = host.advance(repeat)
            assert host.state(purchase).request['op'] == 'charge'
            assert host.advance(repeat) == purchase
            try:
                host.settle(purchase)
            except Denied as error:
                assert str(error) == 'operation unavailable'
            else:
                raise AssertionError('new charge must lack host authority after cancellation')
            terminal = host.advance(purchase)
            assert isinstance(terminal, Finished)
            assert terminal.outcome['status'] == 'cancelled' and terminal.outcome['reason'] == ''
            assert terminal.outcome['error'] is None
            assert json.loads(terminal.outcome['value']) == [
                ['delivered', ''],
                ['denied', 'operation unavailable'],
            ]
            assert host.advance(purchase) == terminal
            assert host.service.count('charge') == 0 and host.service.count('release') == 1
            results[backend] = record(host, terminal)
        finally:
            host.journal.close()
            host.service.close()
    (args.output / 'report.json').write_text(json.dumps(results, indent=2) + '\n')
    print('PASS 2 catch-after-delivery host defense scenarios')


if __name__ == '__main__':
    main()
