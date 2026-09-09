"""Exercise deployment failure ordering with isolated files and fake system tools.
No SSH, supplier calls, real services or databases are used.
"""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).with_name('activate.sh').read_text()
MOCK = r'''#!/usr/bin/env python3
import os, sys, pathlib, shutil
name = pathlib.Path(sys.argv[0]).name
args = sys.argv[1:]
with open(os.environ['EVENTS'], 'a') as out:
    out.write(name + ' ' + ' '.join(args) + '\n')
mode = os.environ['FAIL_MODE']
if name == 'runuser':
    if 'migrate' in args:
        assert os.environ['DATABASE_URL'].startswith('postgres://shapon_migrator:')
        sys.exit(1 if mode == 'migration' else 0)
    if any('shapontravels-backup' in arg for arg in args):
        sys.exit(1 if mode == 'backup' else 0)
    if 'psql' in args:
        sys.stdin.read()
        sys.exit(1 if mode == 'grants' else 0)
if name == 'curl':
    sys.exit(1 if mode == 'health' else 0)
if name == 'install':
    if '-d' in args:
        pathlib.Path(args[-1]).mkdir(parents=True, exist_ok=True)
    else:
        shutil.copyfile(args[-2], args[-1])
        pathlib.Path(args[-1]).chmod(0o750)
if name == 'mv':
    os.replace(args[-2], args[-1])
'''


class DeploymentTests(unittest.TestCase):
    def run_deploy(self, mode='', sha='a' * 40):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        root = Path(temp.name)
        mock_bin = root / 'bin'
        mock_bin.mkdir()
        for cmd in ('flock', 'systemctl', 'runuser', 'curl', 'sleep', 'install', 'mv'):
            target = mock_bin / cmd
            target.write_text(MOCK)
            target.chmod(0o755)
        for directory in ('run/lock', 'etc/shapontravels', 'opt/shapontravels',
                          'usr/local/sbin', 'home/deploy/incoming/' + 'a' * 40):
            (root / directory).mkdir(parents=True, exist_ok=True)
        (root / 'etc/shapontravels/.env').write_text('APP_ENV=production\n')
        (root / 'etc/shapontravels/migration-database-url').write_text(
            'postgres://shapon_migrator:test@127.0.0.1:5432/shapontravels\n')
        backup = root / 'usr/local/sbin/shapontravels-backup'
        backup.touch()
        backup.chmod(0o755)
        (root / ('home/deploy/incoming/' + 'a' * 40 + '/shapontravels-api')).write_text('new binary')
        active = root / 'opt/shapontravels/shapontravels-api'
        active.write_text('old binary')
        # Transform only this temporary copy; production has no test bypass flags.
        script = SCRIPT.replace('export PATH=/usr/sbin:/usr/bin:/sbin:/bin', '')
        script = script.replace('[[ $EUID == 0 ]]', 'true')
        for prefix in ('/home/deploy', '/opt/shapontravels', '/etc/shapontravels',
                       '/run/lock', '/usr/local/sbin'):
            script = script.replace(prefix, str(root) + prefix)
        events = root / 'events'
        result = subprocess.run(['bash', '-c', script, 'activate', sha], text=True,
                                capture_output=True, env={**os.environ,
                                'PATH': str(mock_bin) + ':' + os.environ['PATH'],
                                'EVENTS': str(events), 'FAIL_MODE': mode})
        log = events.read_text() if events.exists() else ''
        self.assertNotIn('postgres://shapon_migrator:test', result.stdout + result.stderr)
        return result, log, active

    def test_success_backs_up_before_stop_and_migrates_before_start(self):
        result, log, active = self.run_deploy()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(active.is_symlink())
        self.assertEqual(active.read_text(), 'new binary')
        self.assertLess(log.index('shapontravels-backup'), log.index('systemctl stop'))
        self.assertLess(log.index(' migrate'), log.index('systemctl start'))

    def test_backup_failure_keeps_old_service_untouched(self):
        result, log, active = self.run_deploy('backup')
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn('systemctl stop', log)
        self.assertEqual(active.read_text(), 'old binary')

    def test_migration_or_grant_failure_does_not_activate_release(self):
        for mode in ('migration', 'grants'):
            with self.subTest(mode=mode):
                result, log, active = self.run_deploy(mode)
                self.assertNotEqual(result.returncode, 0)
                self.assertNotIn('systemctl start', log)
                self.assertEqual(active.read_text(), 'old binary')
                self.assertIn('Service remains stopped', result.stderr)

    def test_unhealthy_release_is_stopped_without_unsafe_schema_rollback(self):
        result, log, _ = self.run_deploy('health')
        self.assertNotEqual(result.returncode, 0)
        self.assertGreater(log.rindex('systemctl stop'), log.index('systemctl start'))

    def test_invalid_release_id_runs_no_commands(self):
        result, log, _ = self.run_deploy(sha='../../bad;echo injected')
        self.assertEqual(result.returncode, 2)
        self.assertEqual(log, '')


if __name__ == '__main__':
    unittest.main()
