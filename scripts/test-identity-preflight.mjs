// Hermetic CLI contract test. Every HTTP request stays on this loopback server.
import assert from 'node:assert/strict';
import http from 'node:http';
import { once } from 'node:events';
import { spawn } from 'node:child_process';
import { mkdtemp, readFile, stat } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { validateReport } from './identity-preflight.mjs';

const tmp = await mkdtemp(path.join(os.tmpdir(), 'identity-preflight-contract-'));
const token = `stio_${'a'.repeat(43)}`;
const queues = ['creates', 'invitations', 'deletions', 'effects', 'events', 'mail', 'assets', 'wallet_notifications', 'wallet_deliveries', 'business_deliveries', 'wallet_requests', 'wallet_reservations', 'bookings', 'ticket_issues', 'cancellations'];
const mappings = ['agency_wallet_missing', 'unmapped_wallet_owners', 'unmapped_client_subjects', 'unmapped_staff_subjects', 'unmapped_draft_subjects', 'unmapped_booking_creators', 'agency_client_wallet_mismatch', 'active_agency_users_without_membership'];
const tables = ['portal_identity_control', 'portal_users', 'portal_agencies', 'portal_agency_memberships', 'portal_agency_wallets', 'wallet_owners', 'wallet_client_links', 'api_clients', 'portal_staff_clients', 'portal_hold_drafts', 'flight_bookings'];
const fresh = () => ({ format_version: 1, mapping_digest: 'a'.repeat(64), observed_at: new Date().toISOString(), authority_mode: 'staged', schema_ready: true, bootstrap_ready: true,
  selected_operator_active_superadmin: true, operator_provider_verified: false, maintenance_enabled: true, worker_paused: true,
  backlog: Object.fromEntries(queues.map(k => [k, { pending: 0, uncertain: 0 }])), mapping_issues: Object.fromEntries(mappings.map(k => [k, 0])),
  mapping_fingerprints: Object.fromEntries(tables.map(k => [k, { rows: 1, sha256: 'a'.repeat(64) }])), database_review_clear: true, activation_ready: false,
  blockers: ['explicit_operator_activation_required', 'selected_provider_operator_not_verified', 'target_backup_restore_evidence_required', 'old_writers_shutdown_evidence_required', 'deployment_configuration_review_required', 'live_cutover_not_authorized'], secret: 'MUST_NOT_APPEAR' });
let body = fresh(), status = 200, requests = 0;
const server = http.createServer(async (req, res) => {
  assert.equal(req.url, '/admin/portal-identity/preflight'); assert.equal(req.method, 'POST');
  assert.equal(req.headers.authorization, `Bearer ${token}`);
  let raw = ''; for await (const chunk of req) raw += chunk;
  assert.deepEqual(JSON.parse(raw), { clerk_user_id: 'user_selected' }); requests++;
  res.writeHead(status, { 'Content-Type': 'application/json' }); res.end(JSON.stringify(body));
});
server.listen(0, '127.0.0.1'); await once(server, 'listening');
const origin = `http://127.0.0.1:${server.address().port}`;
const program = fileURLToPath(new URL('./identity-preflight.mjs', import.meta.url));
async function run(name, extra = [], overrides = {}) {
  const args = [program, '--origin', origin, '--operator', 'user_selected', '--output', path.join(tmp, name), ...extra];
  const child = spawn(process.execPath, args, { env: { PATH: process.env.PATH, PORTAL_IDENTITY_OPERATOR_TOKEN: token, ...overrides } });
  let output = ''; child.stdout.on('data', chunk => output += chunk); child.stderr.on('data', chunk => output += chunk);
  const [code] = await once(child, 'close');
  assert(!output.includes(token)); assert(!output.includes('MUST_NOT_APPEAR'));
  return { code, output };
}
try {
  assert.equal((await run('first.json')).code, 0);
  const saved = await readFile(path.join(tmp, 'first.json'), 'utf8');
  assert(!saved.includes(token)); assert(!saved.includes('MUST_NOT_APPEAR')); assert(!saved.includes('user_selected'));
  assert.equal((await stat(path.join(tmp, 'first.json'))).mode & 0o777, 0o600);
  assert.equal(JSON.parse(saved).review.activation_performed, false);
  assert.equal((await run('first.json')).code, 1, 'no overwrite');
  assert.equal(await readFile(path.join(tmp, 'first.json'), 'utf8'), saved);
  assert.equal((await run('same.json', ['--compare', path.join(tmp, 'first.json')])).code, 0);
  body.mapping_fingerprints.portal_users.sha256 = 'b'.repeat(64);
  assert.equal((await run('drift.json', ['--compare', path.join(tmp, 'first.json')])).code, 2);
  body = fresh(); body.maintenance_enabled = body.worker_paused = false;
  assert.equal((await run('not-paused.json')).code, 2);
  body = fresh(); body.backlog.mail = { pending: 1, uncertain: 1 }; body.database_review_clear = false; body.blockers.push('unresolved_mail');
  assert.equal((await run('pending.json')).code, 2);
  for (const mutate of [v => v.activation_ready = true, v => delete v.backlog.assets, v => v.mapping_issues.unmapped_wallet_owners = -1, v => v.backlog.mail.pending = 1, v => v.mapping_fingerprints.portal_users.sha256 = 'bad', v => v.observed_at = '2000-01-01T00:00:00Z', v => v.worker_paused = false, v => v.blockers = [token]]) {
    body = fresh(); mutate(body);
    assert.throws(() => validateReport(body));
    assert.equal((await run(`bad-${requests}.json`)).code, 1);
  }
  status = 503; body = { error: token };
  assert.equal((await run('unavailable.json')).code, 1);
  status = 200; body = fresh();
  const before = requests;
  assert.equal((await run('bad-token.json', [], { PORTAL_IDENTITY_OPERATOR_TOKEN: 'bad' })).code, 1);
  assert.equal((await run('duplicate.json', ['--origin', 'https://remote.example.invalid'])).code, 1);
  assert.equal(requests, before, 'invalid config must fail before any HTTP request');
  console.log('Preflight CLI contract passed: read-only, private/no-overwrite output, strict response, drift, pending work and maintenance checks.');
} finally { server.close(); await once(server, 'close'); }
