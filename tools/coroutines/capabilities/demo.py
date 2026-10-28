"""Real native/Wasm replay, revocation and cancellation against the durable host journal."""

import argparse
import hashlib
import json
from pathlib import Path

from host import Finished, Host
from journal import Denied, FakeService, Journal

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parents[2]


def provision(directory):
    """Create independent host/service stores and pregrant narrow demo authority."""
    journal = Journal(directory / 'host.db')
    service = FakeService(directory / 'service.db')
    run, lineage = journal.create_run('tenant-a')
    handles = [
        journal.grant(run, 'reserve', 'SKU-123', 2),
        journal.grant(run, 'charge', 'account-1', 42),
        journal.grant(run, 'release', 'hold-1'),
    ]
    return Host(journal, service, 'tenant-a', run, lineage), handles


def denied(host, checkpoint):
    """Require a host rejection, not a fabricated guest failure."""
    try:
        host.settle(checkpoint)
    except Denied as error:
        return str(error)
    raise AssertionError('expected denial')


def run(backend, directory):
    """Replay actual saved Monty bytes using fresh runtime processes on every advance."""
    directory.mkdir(parents=True, exist_ok=False)
    results = {}
    for scenario in ('replay', 'revoke', 'cancel_body', 'cancel_cleanup'):
        folder = directory / scenario
        folder.mkdir()
        host, handles = provision(folder)
        try:
            reserve = host.start(backend, handles, after_cleanup=scenario == 'cancel_cleanup')
            payment = host.advance(reserve)
            cuts = [reserve, payment]
            if scenario == 'replay':
                cleanup = host.advance(payment)
                # Restore the identical payment snapshot, replay settlement, execute VM again.
                replay = host.advance(payment)
                assert cleanup == replay
                assert host.service.count('charge') == 1
                terminal = host.advance(cleanup)
                assert isinstance(terminal, Finished) and terminal.outcome['status'] == 'succeeded'
                host.journal.revoke(handles[1])
                assert host.settle(payment)['receipt']
                cuts.append(cleanup)
            elif scenario == 'revoke':
                host.journal.revoke(handles[1])
                results['revoked_denial'] = denied(host, payment)
                cleanup = host.advance(payment)
                terminal = host.advance(cleanup)
                assert terminal.outcome['status'] == 'failed'
                assert host.service.count('charge') == 0 and host.service.count('release') == 1
                cuts.append(cleanup)
            elif scenario == 'cancel_body':
                cleanup = host.advance(payment, cancel='stop')
                results['cancelled_denial'] = denied(host, payment)
                terminal = host.advance(cleanup)
                assert terminal.outcome['status'] == 'cancelled' and terminal.outcome['reason'] == 'stop'
                assert host.service.count('charge') == 0 and host.service.count('release') == 1
                assert host.settle(cleanup) == {'released': 'hold-1'}
                # Roll back to before the guest ever observed cancellation. The host
                # reasserts its durable first reason while replaying settled reserve.
                rollback_terminal = host.advance(reserve)
                assert isinstance(rollback_terminal, Finished)
                assert rollback_terminal.outcome['status'] == 'cancelled'
                assert rollback_terminal.outcome['reason'] == 'stop'
                results['rollback_terminal'] = rollback_terminal.outcome
                assert host.service.count('charge') == 0
                cuts.append(cleanup)
            else:
                cleanup = host.advance(payment)
                protected = host.advance(cleanup, cancel='stop')
                # Native cancellation preserves this wait; no guest "cleanup" label is trusted.
                assert host.state(protected).request == host.state(cleanup).request
                terminal = host.advance(protected)
                # The VM delivers before emitting a second purchase request.
                assert isinstance(terminal, Finished)
                assert terminal.outcome['status'] == 'cancelled' and terminal.outcome['reason'] == 'stop'
                assert host.service.count('charge') == 1 and host.service.count('release') == 1
                assert host.settle(cleanup) == {'released': 'hold-1'}
                cuts.extend([cleanup, protected])
            results[scenario] = {
                'terminal': terminal.outcome,
                'charge_count': host.service.count('charge'),
                'release_count': host.service.count('release'),
                'snapshots': [
                    {
                        'checkpoint': cut,
                        'sha256': hashlib.sha256(host.state(cut).snapshot).hexdigest(),
                        'request': host.state(cut).request,
                    }
                    for cut in cuts
                ],
                'start_evidence': host.evidence,
            }
        finally:
            host.journal.close()
            host.service.close()
    return results


def main():
    """Keep evidence and SQLite snapshots without overwriting a previous execution."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--backend', choices=('native', 'wasm'), required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    results = run(args.backend, args.output)
    (args.output / 'report.json').write_text(json.dumps(results, indent=2) + '\n')
    print(json.dumps({'backend': args.backend, 'scenarios': 4, 'status': 'PASS'}))


if __name__ == '__main__':
    main()
