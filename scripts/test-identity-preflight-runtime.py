#!/usr/bin/env python3
"""Exercise the actual binary in maintenance against an existing SYNTHETIC DB.
No dotenv, migrations, bootstrap, data writes or external provider credentials.
"""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import socket
import subprocess
import tempfile
import time
from urllib.error import HTTPError, URLError
from urllib.parse import urlparse
from urllib.request import Request, urlopen


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--database', required=True)
    parser.add_argument('--pg-bin', type=Path, required=True)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    u = urlparse(args.database)
    if u.scheme not in ('postgres', 'postgresql') or u.hostname != '127.0.0.1' or not re.fullmatch(r'/[a-z0-9_]+_identity_test', u.path) or u.query or u.fragment:
        parser.error('Only an explicitly disposable loopback _identity_test database is accepted')
    binary = args.binary.resolve()
    output = args.output.resolve()
    output.mkdir(mode=0o700, parents=True, exist_ok=False)
    psql = (args.pg_bin / 'psql').resolve()
    node = shutil.which('node')
    if not node:
        raise RuntimeError('Node is required')
    program = Path(__file__).with_name('identity-preflight.mjs').resolve()
    env = {'PATH': os.environ.get('PATH', '/usr/bin:/bin'), 'DATABASE_URL': args.database,
           'APP_ENV': 'test', 'PORTAL_IDENTITY_MODE': 'staged', 'PORTAL_IDENTITY_MAINTENANCE': 'true',
           'PORTAL_IDENTITY_BRIDGE_TOKEN': 'stib_' + 'a' * 43,
           'PORTAL_IDENTITY_OPERATOR_TOKEN': 'stio_' + 'b' * 43,
           'PORTAL_IDENTITY_OPERATOR_ID': 'synthetic_preflight_review',
           'PORTAL_IDENTITY_CLERK_SECRET_KEY': 'sk_test_synthetic_unused',
           'PORTAL_IDENTITY_PROVIDER_WRITES': 'disabled'}

    def query(sql):
        result = subprocess.run([str(psql), '-X', '-At', '--dbname', args.database, '-v', 'ON_ERROR_STOP=1', '-c', sql], env={'PATH': env['PATH']}, capture_output=True, text=True, timeout=30)
        if result.returncode:
            raise RuntimeError('Synthetic database inspection failed')
        return result.stdout.strip()

    def snapshot():
        result = {}
        for table in query("SELECT tablename FROM pg_tables WHERE schemaname='public' ORDER BY tablename").splitlines():
            assert re.fullmatch('[a-z0-9_]+', table)
            result[table] = query("SELECT count(*)::text||':'||md5(coalesce(string_agg(to_jsonb(t)::text,E'\\n' ORDER BY to_jsonb(t)::text),'')) FROM " + table + ' t')
        result['sequences'] = query("SELECT coalesce(jsonb_agg(jsonb_build_array(sequencename,last_value) ORDER BY sequencename),'[]'::jsonb) FROM pg_sequences WHERE schemaname='public'")
        return result

    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        port = sock.getsockname()[1]
    origin = f'http://127.0.0.1:{port}'
    env['APP_BIND'] = f'127.0.0.1:{port}'
    before = snapshot()
    with tempfile.TemporaryDirectory(prefix='identity-preflight-runtime-') as cwd, (output / 'server.log').open('x') as log:
        process = subprocess.Popen([str(binary), 'serve'], cwd=cwd, env=env, stdout=log, stderr=log)
        try:
            for _ in range(100):
                if process.poll() is not None:
                    raise RuntimeError('Isolated Rust startup failed; inspect retained log')
                try:
                    with urlopen(origin + '/health/live', timeout=1) as response:
                        if response.status == 200:
                            break
                except (URLError, TimeoutError):
                    time.sleep(0.1)
            else:
                raise RuntimeError('Isolated Rust startup timed out')
            for method, path in [('GET', '/health/ready'), ('POST', '/admin/login'), ('POST', '/admin/portal-identity/bootstrap'), ('POST', '/api/Search')]:
                try:
                    urlopen(Request(origin + path, method=method), timeout=5)
                    raise AssertionError('Maintenance admitted a blocked route')
                except HTTPError as error:
                    assert error.code == 503
                    assert json.loads(error.read())['error'] == 'IDENTITY_MAINTENANCE'
            req = Request(origin + '/admin/portal-identity/readiness', data=b'{}', headers={'Authorization': 'Bearer ' + env['PORTAL_IDENTITY_BRIDGE_TOKEN'], 'Content-Type': 'application/json'}, method='POST')
            with urlopen(req, timeout=10) as response:
                ready = json.load(response)
            assert ready['maintenance_enabled'] and ready['worker_paused'] and ready['schema_ready']
            assert not ready['provider_writes_enabled'] and not ready['live_activation_available']
            for index in [1, 2]:
                cmd = [node, str(program), '--origin', origin, '--operator', 'user_root', '--output', str(output / f'review-{index}.json')]
                if index == 2:
                    cmd += ['--compare', str(output / 'review-1.json')]
                run = subprocess.run(cmd, env={'PATH': env['PATH'], 'PORTAL_IDENTITY_OPERATOR_TOKEN': env['PORTAL_IDENTITY_OPERATOR_TOKEN']}, capture_output=True, text=True, timeout=25)
                if run.returncode not in [0, 2]:
                    raise RuntimeError('Actual HTTP preflight CLI failed')
                report = json.loads((output / f'review-{index}.json').read_text())['review']
                assert report['report']['maintenance_enabled'] and report['activation_performed'] is False
                assert report['report']['activation_ready'] is False
                assert run.returncode == (0 if report['report']['database_review_clear'] else 2)
                if index == 2:
                    assert report['mapping_drift'] is False
            # Allow a real worker tick; paused services must leave every row alone.
            time.sleep(6)
        finally:
            process.terminate()
            try:
                process.wait(timeout=15)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
    assert snapshot() == before, 'Maintenance/preflight changed database data or sequences'
    evidence = {'database': u.path[1:], 'source_unchanged': True, 'public_tables_verified': len(before) - 1,
                'actual_binary_maintenance_verified': True, 'actual_http_cli_verified': True,
                'backlog': ready['backlog'], 'activation_performed': False, 'provider_writes_enabled': False}
    (output / 'evidence.json').write_text(json.dumps(evidence, indent=2) + '\n')
    print(json.dumps(evidence, indent=2))


if __name__ == '__main__':
    main()
