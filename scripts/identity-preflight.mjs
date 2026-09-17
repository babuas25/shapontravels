// Read-only operator review. No dotenv, bootstrap, migration, provider call or cutover.
import { createHash } from 'node:crypto';
import { readFile, writeFile } from 'node:fs/promises';
import { pathToFileURL } from 'node:url';

const hash = value => createHash('sha256').update(value).digest('hex');
const digest = value => hash(JSON.stringify(value));
const invalid = () => { throw new Error('Invalid identity preflight response. No activation was performed.'); };
const boolean = value => typeof value === 'boolean';
const count = value => Number.isSafeInteger(value) && value >= 0;
export function validateReport(v) {
  if (!v || v.format_version !== 1 || !['staged','canonical'].includes(v.authority_mode) || v.activation_ready !== false || v.operator_provider_verified !== false
      || !['schema_ready', 'bootstrap_ready', 'selected_operator_active_superadmin', 'maintenance_enabled', 'worker_paused', 'database_review_clear'].every(k => boolean(v[k]))
      || v.schema_ready !== true || v.maintenance_enabled !== v.worker_paused
      || !/^[a-f0-9]{64}$/.test(v.mapping_digest ?? '')
      || typeof v.observed_at !== 'string' || !Number.isFinite(Date.parse(v.observed_at))
      || Math.abs(Date.now() - Date.parse(v.observed_at)) > 300000) invalid();
  const queues = ['creates', 'invitations', 'deletions', 'effects', 'events', 'mail', 'assets', 'wallet_notifications', 'wallet_deliveries', 'wallet_requests', 'wallet_reservations', 'bookings', 'ticket_issues', 'cancellations'];
  if (!v.backlog || Object.keys(v.backlog).length !== queues.length || queues.some(k => !v.backlog[k] || !count(v.backlog[k].pending) || !count(v.backlog[k].uncertain) || v.backlog[k].uncertain > v.backlog[k].pending)) invalid();
  const mappings = ['agency_wallet_missing', 'unmapped_wallet_owners', 'unmapped_client_subjects', 'unmapped_staff_subjects', 'unmapped_draft_subjects', 'unmapped_booking_creators', 'agency_client_wallet_mismatch', 'active_agency_users_without_membership'];
  if (!v.mapping_issues || Object.keys(v.mapping_issues).length !== mappings.length || mappings.some(k => !count(v.mapping_issues[k]))) invalid();
  const tables = ['portal_identity_control', 'portal_users', 'portal_agencies', 'portal_agency_memberships', 'portal_agency_wallets', 'wallet_owners', 'wallet_client_links', 'api_clients', 'portal_staff_clients', 'portal_hold_drafts', 'flight_bookings'];
  if (!v.mapping_fingerprints || Object.keys(v.mapping_fingerprints).length !== tables.length || tables.some(k => !count(v.mapping_fingerprints[k]?.rows) || !/^[a-f0-9]{64}$/.test(v.mapping_fingerprints[k]?.sha256 ?? ''))) invalid();
  const clear = v.bootstrap_ready && v.selected_operator_active_superadmin && queues.every(k => v.backlog[k].pending === 0) && mappings.every(k => v.mapping_issues[k] === 0);
  if (v.database_review_clear !== clear || !Array.isArray(v.blockers) || v.blockers.length > 100 || v.blockers.some(s => typeof s !== 'string' || !/^[a-z_]{1,100}$/.test(s))) invalid();
  const expectedBlockers = ['explicit_operator_activation_required', 'selected_provider_operator_not_verified', 'target_backup_restore_evidence_required', 'old_writers_shutdown_evidence_required', 'deployment_configuration_review_required', 'live_cutover_not_authorized'];
  if (!v.bootstrap_ready) expectedBlockers.push('bootstrap_required');
  if (!v.selected_operator_active_superadmin) expectedBlockers.push('selected_operator_not_active_superadmin');
  for (const k of queues) if (v.backlog[k].pending) expectedBlockers.push(`unresolved_${k}`);
  for (const k of mappings) if (v.mapping_issues[k]) expectedBlockers.push(`mapping_review_${k}`);
  if (JSON.stringify([...v.blockers].sort()) !== JSON.stringify(expectedBlockers.sort())) invalid();
  // Whitelist fields: even a malformed upstream cannot echo credentials/PII to disk.
  const work = Object.fromEntries(queues.map(k => [k, { pending: v.backlog[k].pending, uncertain: v.backlog[k].uncertain }]));
  const fingerprints = Object.fromEntries(tables.map(k => [k, { rows: v.mapping_fingerprints[k].rows, sha256: v.mapping_fingerprints[k].sha256 }]));
  return { format_version: 1, observed_at: v.observed_at, mapping_digest: v.mapping_digest, authority_mode: v.authority_mode, schema_ready: true, bootstrap_ready: v.bootstrap_ready,
    selected_operator_active_superadmin: v.selected_operator_active_superadmin, operator_provider_verified: false,
    maintenance_enabled: v.maintenance_enabled, worker_paused: v.worker_paused, backlog: work,
    mapping_issues: Object.fromEntries(mappings.map(k => [k, v.mapping_issues[k]])), mapping_fingerprints: fingerprints,
    database_review_clear: clear, activation_ready: false, blockers: v.blockers };
}

async function boundedJson(response) {
  if (!response.ok) throw new Error(`Identity preflight failed (HTTP ${response.status}). No activation was performed.`);
  const reader = response.body?.getReader(); if (!reader) invalid();
  let size = 0; const chunks = [];
  try { for (;;) { const { value, done } = await reader.read(); if (done) break; size += value.length; if (size > 262144) { await reader.cancel(); invalid(); } chunks.push(value); } }
  finally { reader.releaseLock(); }
  try { return JSON.parse(Buffer.concat(chunks).toString('utf8')); } catch { invalid(); }
}

export async function main(args = process.argv.slice(2)) {
  const options = {};
  for (let i = 0; i < args.length; i += 2) {
    if (!['--origin', '--operator', '--output', '--compare'].includes(args[i]) || !args[i + 1] || args[i] in options) throw new Error('Usage: node scripts/identity-preflight.mjs --origin ORIGIN --operator user_SELECTED --output NEW_FILE [--compare PRIOR_FILE]');
    options[args[i]] = args[i + 1];
  }
  if (!options['--output'] || !/^user_[A-Za-z0-9_]{1,123}$/.test(options['--operator'] ?? '')) throw new Error('An explicitly selected operator and new output file are required.');
  let url;
  try {
    url = new URL(options['--origin']);
    if ((url.protocol !== 'https:' && !(url.protocol === 'http:' && ['127.0.0.1', 'localhost'].includes(url.hostname))) || url.pathname !== '/' || url.username || url.password || url.search || url.hash) throw Error();
  } catch { throw new Error('Use an explicit HTTPS origin or loopback HTTP fixture, without credentials or a path.'); }
  const token = process.env.PORTAL_IDENTITY_OPERATOR_TOKEN;
  if (!/^stio_[A-Za-z0-9_-]{43}$/.test(token ?? '')) throw new Error('A dedicated operator credential is required in PORTAL_IDENTITY_OPERATOR_TOKEN.');
  let prior;
  if (options['--compare']) {
    try { prior = JSON.parse(await readFile(options['--compare'], 'utf8')); }
    catch { throw new Error('Prior review file could not be read.'); }
    if (!prior?.review || prior.sha256 !== digest(prior.review) || prior.review.target_origin !== url.origin || prior.review.operator_sha256 !== hash(options['--operator']) || !prior.review.report?.mapping_fingerprints) throw new Error('Prior review is invalid or belongs to a different target/operator.');
  }
  let report;
  try {
    const response = await fetch(`${url.origin}/admin/portal-identity/preflight`, { method: 'POST', cache: 'no-store', redirect: 'error', signal: AbortSignal.timeout(18000),
      headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' }, body: JSON.stringify({ clerk_user_id: options['--operator'] }) });
    report = validateReport(await boundedJson(response));
  } catch (error) {
    if (error instanceof Error && /^(Invalid identity preflight response|Identity preflight failed)/.test(error.message)) throw error;
    throw new Error('Identity preflight could not be verified. No activation was performed.');
  }
  const drift = prior ? digest(prior.review.report.mapping_fingerprints) !== digest(report.mapping_fingerprints) : null;
  const review = { format_version: 1, target_origin: url.origin, operator_sha256: hash(options['--operator']), report, mapping_drift: drift, activation_performed: false };
  try { await writeFile(options['--output'], JSON.stringify({ review, sha256: digest(review) }, null, 2) + '\n', { flag: 'wx', mode: 0o600 }); }
  catch { throw new Error('Could not create a new private review file. Existing files are never overwritten.'); }
  console.log(JSON.stringify({ review_saved: true, database_review_clear: report.database_review_clear, maintenance_enabled: report.maintenance_enabled, mapping_drift: drift, activation_ready: false, activation_performed: false, blockers: report.blockers }, null, 2));
  return report.database_review_clear && report.maintenance_enabled && drift !== true ? 0 : 2;
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().then(code => { process.exitCode = code; }).catch(error => { console.error(error.message); process.exitCode = 1; });
}
