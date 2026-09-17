"""Check release-helper compatibility before remote mutations; no real SSH."""
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

SOURCE = Path(__file__).parent
MOCK = r'''#!/usr/bin/env python3
import os, pathlib, sys
name = pathlib.Path(sys.argv[0]).name
with open(os.environ['EVENTS'], 'a') as out:
    out.write(name + ' ' + sys.argv[-1] + '\n')
if name == 'ssh' and sys.argv[-1].startswith('sha256sum '):
    if os.environ['MODE'] == 'missing':
        sys.exit(1)
    value = os.environ['HELPER_SHA'] if os.environ['MODE'] == 'match' else '0' * 64
    print(value + '  /usr/local/sbin/shapontravels-activate')
'''


class SendTests(unittest.TestCase):
    def run_send(self, mode):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            scripts = root / 'scripts' / 'deploy'
            scripts.mkdir(parents=True)
            for name in ('send.sh', 'activate.sh'):
                shutil.copyfile(SOURCE / name, scripts / name)
            dist = root / 'dist'
            dist.mkdir()
            binary = b'synthetic release binary'
            (dist / 'shapontravels-api').write_bytes(binary)
            (dist / 'SHA256SUMS').write_text(
                hashlib.sha256(binary).hexdigest() + '  shapontravels-api\n')
            mock_bin = root / 'bin'
            mock_bin.mkdir()
            for name in ('ssh', 'scp'):
                command = mock_bin / name
                command.write_text(MOCK)
                command.chmod(0o755)
            # macOS supplies shasum; CI supplies sha256sum.
            if not shutil.which('sha256sum'):
                command = mock_bin / 'sha256sum'
                command.write_text('#!/bin/sh\nexec shasum -a 256 "$@"\n')
                command.chmod(0o755)
            events = root / 'events'
            result = subprocess.run(
                ['bash', 'scripts/deploy/send.sh'], cwd=root, capture_output=True,
                text=True, env={**os.environ, 'PATH': str(mock_bin) + ':' + os.environ['PATH'],
                'VPS_HOST': 'server.example.invalid', 'VPS_SSH_PRIVATE_KEY': 'fake-key',
                'VPS_KNOWN_HOSTS': 'fake-host', 'DEPLOY_SHA': 'a' * 40,
                'EVENTS': str(events), 'MODE': mode,
                'HELPER_SHA': hashlib.sha256((scripts / 'activate.sh').read_bytes()).hexdigest()})
            return result, events.read_text()

    def test_matching_helper_allows_upload_and_activation(self):
        result, events = self.run_send('match')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertLess(events.index('sha256sum /usr/local/sbin/'), events.index('mkdir -p'))
        self.assertIn('scp ', events)
        self.assertIn('sudo -n /usr/local/sbin/shapontravels-activate', events)

    def test_stale_or_unreadable_helper_does_not_mutate_server(self):
        for mode in ('stale', 'missing'):
            with self.subTest(mode=mode):
                result, events = self.run_send(mode)
                self.assertNotEqual(result.returncode, 0)
                self.assertNotIn('mkdir ', events)
                self.assertNotIn('scp ', events)
                self.assertNotIn('sudo ', events)


if __name__ == '__main__':
    unittest.main()
