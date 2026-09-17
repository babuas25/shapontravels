#!/usr/bin/env node
// Resume this workspace's existing local portal without changing its credentials.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { homedir } from 'node:os';
import { fileURLToPath } from 'node:url';
import { spawn, spawnSync } from 'node:child_process';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
// Once localhost is cut over, never restart the old identity authority.
if (fs.existsSync(path.join(root, '.local/identity-local/config.json'))) {
  await import('./identity-local-dev.mjs');
} else {
const frontend = path.resolve(root, '../shopontravels');
const credentials = path.join(frontend, 'outputs/api-management-dashboard-local/bridge-credentials.json');
assert(fs.existsSync(credentials), 'Existing portal credentials missing. Follow docs/LOCAL_PORTAL.md to configure a new installation.');
const saved = JSON.parse(fs.readFileSync(credentials, 'utf8'));
const db = new URL(saved.database);
assert(['postgres:', 'postgresql:'].includes(db.protocol) && db.hostname === '127.0.0.1' && db.port === '55439' && /^\/api_portal_local_[a-z0-9_]+$/.test(db.pathname) && !db.search && !db.hash, 'This launcher only resumes the existing loopback portal database.');
assert(typeof saved.password === 'string' && saved.password.length >= 64, 'Saved portal credentials are invalid.');
assert(process.argv.length === 2, 'Usage: node scripts/portal-dev.mjs');

const alreadyRunning = await fetch('http://127.0.0.1:18081/health/live', { signal: AbortSignal.timeout(1500) }).then(r => r.ok).catch(() => false);
if (alreadyRunning) {
  console.log('A backend is already running on port 18081. Stop it with Ctrl+C in its terminal before restarting.');
  process.exit(0);
}
function run(command, args, env = process.env, capture = false) {
  const result = spawnSync(command, args, { cwd: root, env, stdio: capture ? ['ignore', 'pipe', 'pipe'] : 'inherit', encoding: 'utf8' });
  if (result.error || result.status !== 0) throw new Error(`${path.basename(command)} failed. Check the local setup and retry.`);
  return result.stdout?.trim();
}
const pg = path.join(root, '.local/pgsql/bin');
const data = path.join(root, '.local/tier-check/data');
const ready = spawnSync(path.join(pg, 'pg_isready'), ['-h', '127.0.0.1', '-p', '55439'], { stdio: 'ignore' });
if (ready.status !== 0) {
  assert(fs.existsSync(path.join(data, 'PG_VERSION')), 'Local PostgreSQL cluster missing; see docs/LOCAL_PORTAL.md.');
  run(path.join(pg, 'pg_ctl'), ['-D', data, '-l', path.join(root, '.local/tier-check/postgres.log'), '-o', '-h 127.0.0.1 -p 55439', '-w', 'start']);
}
const pgEnv = { ...process.env, PGHOST: db.hostname, PGPORT: db.port, PGUSER: decodeURIComponent(db.username), PGPASSWORD: decodeURIComponent(db.password), PGDATABASE: db.pathname.slice(1) };
const installed = Number(run(path.join(pg, 'psql'), ['-X', '-At', '-v', 'ON_ERROR_STOP=1', '-c', 'SELECT COALESCE(max(version),0) FROM _sqlx_migrations WHERE success'], pgEnv, true));
const required = Math.max(...fs.readdirSync(path.join(root, 'migrations')).filter(name => /^\d+_.+\.sql$/.test(name)).map(name => Number(name.split('_')[0])));
if (installed < required) {
  const backup = path.join(root, '.local', `portal-before-migration-${new Date().toISOString().replaceAll(':', '-')}.dump`);
  run(path.join(pg, 'pg_dump'), ['--format=custom', `--file=${backup}`], pgEnv);
  fs.chmodSync(backup, 0o600);
  console.log(`Local database backup saved: ${backup}`);
}
const cargo = path.join(homedir(), '.cargo/bin/cargo');
run(cargo, ['build', '--locked', '--example', 'local_api_management']);
console.log('Starting Rust on http://127.0.0.1:18081 using the saved local portal database.');
console.log('Mode: Triplover UAT Search/Reprice/Hold/PNR; ticket issue disabled. Keep this terminal open.');
const backend = spawn(path.join(root, 'target/debug/examples/local_api_management'), [], {
  cwd: root,
  env: { PATH: process.env.PATH, LOCAL_API_TEST_DATABASE_URL: saved.database, LOCAL_API_TEST_RESUME: '1', LOCAL_API_TEST_BIND: '127.0.0.1:18081', LOCAL_API_UAT_HOLDS: '1' },
  stdio: 'inherit',
});
backend.on('error', () => { console.error('Could not start the local Rust backend.'); process.exitCode = 1; });
for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => backend.kill(signal));
backend.on('exit', (code, signal) => { process.exitCode = signal ? 0 : code ?? 1; });
}
