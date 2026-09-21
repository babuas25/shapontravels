#!/usr/bin/env node
// Generate a private SQL plan for explicitly reviewed Clerk subject remaps.
// This does not load dotenv, contact Clerk, connect to PostgreSQL or apply a
// change. Provider verification must happen before a reviewed manifest is made.
import { createHash, randomUUID } from 'node:crypto';
import { readFileSync, writeFileSync, chmodSync, existsSync } from 'node:fs';
import { resolve } from 'node:path';

const usage = 'Usage: node scripts/identity-subject-remap-plan.mjs --manifest /private/mapping.json --output /private/remap.sql';
const hash = (value) => createHash('sha256').update(value).digest('hex');
const subject = /^user_[A-Za-z0-9_]{1,123}$/;
const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;
const digest = /^[a-f0-9]{64}$/;
const sql = (value) => `'${value.replaceAll("'", "''")}'`;

function fail(message) {
  throw new Error(message);
}

function argumentsFrom(argv) {
  if (argv.length !== 4 || argv[0] !== '--manifest' || argv[2] !== '--output') fail(usage);
  return { manifest: resolve(argv[1]), output: resolve(argv[3]) };
}

function canonicalManifest(value) {
  if (!value || typeof value !== 'object' || Array.isArray(value)) fail('Manifest must be an object.');
  if (value.format_version !== 1 || !['development', 'production'].includes(value.target)
      || typeof value.operator_subject !== 'string' || !subject.test(value.operator_subject)
      || typeof value.provider_evidence_digest !== 'string' || !digest.test(value.provider_evidence_digest)
      || !Array.isArray(value.mappings) || value.mappings.length < 1 || value.mappings.length > 200
      || Object.keys(value).some((key) => !['format_version', 'target', 'operator_subject', 'provider_evidence_digest', 'mappings'].includes(key))) {
    fail('Invalid remap manifest header.');
  }
  const mappings = value.mappings.map((entry) => {
    if (!entry || typeof entry !== 'object' || Array.isArray(entry)
        || Object.keys(entry).some((key) => !['portal_user_id', 'previous_subject', 'current_subject'].includes(key))
        || typeof entry.portal_user_id !== 'string' || !uuid.test(entry.portal_user_id)
        || typeof entry.previous_subject !== 'string' || !subject.test(entry.previous_subject)
        || typeof entry.current_subject !== 'string' || !subject.test(entry.current_subject)
        || entry.previous_subject === entry.current_subject) fail('Invalid remap entry.');
    return { portal_user_id: entry.portal_user_id.toLowerCase(), previous_subject: entry.previous_subject, current_subject: entry.current_subject };
  }).sort((a, b) => a.portal_user_id.localeCompare(b.portal_user_id));
  for (const field of ['portal_user_id', 'previous_subject', 'current_subject']) {
    const values = mappings.map((entry) => entry[field]);
    if (new Set(values).size !== values.length) fail(`Duplicate ${field} in remap manifest.`);
  }
  return { format_version: 1, target: value.target, operator_subject: value.operator_subject, provider_evidence_digest: value.provider_evidence_digest, mappings };
}

try {
  const { manifest, output } = argumentsFrom(process.argv.slice(2));
  if (existsSync(output)) fail('Refusing to overwrite the requested plan output.');
  const parsed = JSON.parse(readFileSync(manifest, 'utf8'));
  const reviewed = canonicalManifest(parsed);
  const mappingDigest = hash(JSON.stringify(reviewed));
  const statements = [
    '-- Generated private subject-remap plan. It contains identifiers, no credentials.',
    'BEGIN;',
    "SET LOCAL lock_timeout = '5s';",
    "SET LOCAL statement_timeout = '30s';",
  ];
  for (const entry of reviewed.mappings) {
    statements.push(
      `SELECT portal_identity_apply_subject_remap(${sql(randomUUID())}::uuid,${sql(entry.portal_user_id)}::uuid,${sql(entry.previous_subject)},${sql(entry.current_subject)},${sql(reviewed.operator_subject)},${sql(mappingDigest)},${sql(reviewed.provider_evidence_digest)});`
    );
  }
  statements.push('COMMIT;', '');
  writeFileSync(output, statements.join('\n'), { encoding: 'utf8', mode: 0o600, flag: 'wx' });
  chmodSync(output, 0o600);
  process.stdout.write(JSON.stringify({ target: reviewed.target, mappings: reviewed.mappings.length, mapping_digest: mappingDigest, output }) + '\n');
} catch (error) {
  process.stderr.write(`${error instanceof Error ? error.message : 'Invalid remap plan.'}\n`);
  process.exitCode = 1;
}
