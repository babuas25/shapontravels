import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import assert from 'node:assert/strict';

const directory = mkdtempSync(join(tmpdir(), 'shapon-identity-remap-'));
try {
  const manifest = join(directory, 'mapping.json');
  const output = join(directory, 'mapping.sql');
  writeFileSync(manifest, JSON.stringify({
    format_version: 1,
    target: 'production',
    operator_subject: 'user_operator',
    provider_evidence_digest: 'a'.repeat(64),
    mappings: [{
      portal_user_id: '00000000-0000-4000-8000-000000000001',
      previous_subject: 'user_previous',
      current_subject: 'user_current',
    }],
  }));
  const result = spawnSync(process.execPath, ['scripts/identity-subject-remap-plan.mjs', '--manifest', manifest, '--output', output], { encoding: 'utf8' });
  assert.equal(result.status, 0, result.stderr);
  const plan = readFileSync(output, 'utf8');
  assert.match(plan, /^BEGIN;$/m);
  assert.match(plan, /portal_identity_apply_subject_remap/);
  assert.match(plan, /^COMMIT;$/m);
  assert.doesNotMatch(plan, /CLERK_SECRET|sk_(live|test)_/);

  const duplicate = join(directory, 'duplicate.json');
  writeFileSync(duplicate, JSON.stringify({
    format_version: 1,
    target: 'production',
    operator_subject: 'user_operator',
    provider_evidence_digest: 'a'.repeat(64),
    mappings: [
      { portal_user_id: '00000000-0000-4000-8000-000000000001', previous_subject: 'user_a', current_subject: 'user_same' },
      { portal_user_id: '00000000-0000-4000-8000-000000000002', previous_subject: 'user_b', current_subject: 'user_same' },
    ],
  }));
  const rejected = spawnSync(process.execPath, ['scripts/identity-subject-remap-plan.mjs', '--manifest', duplicate, '--output', join(directory, 'duplicate.sql')], { encoding: 'utf8' });
  assert.notEqual(rejected.status, 0);
  assert.match(rejected.stderr, /Duplicate current_subject/);
  process.stdout.write('identity subject remap plan checks passed\n');
} finally {
  rmSync(directory, { recursive: true, force: true });
}
