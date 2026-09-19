// Validate synthetic machine responses against the SAME schemas as native APIs.
// Usage: node scripts/verify-import-api-contract.cjs <responses.json>
const fs = require('node:fs');
const assert = require('node:assert/strict');
const path = require('node:path');
const { createRequire } = require('node:module');
const frontendRequire = createRequire(path.resolve(__dirname, '../../shopontravels/package.json'));
const Ajv = frontendRequire('ajv');
const schemas = JSON.parse(fs.readFileSync(path.resolve(__dirname, '../src/client_schemas.json'), 'utf8'));
const captures = JSON.parse(fs.readFileSync(process.argv[2], 'utf8'));
const ajv = new Ajv({ allErrors: true, schemaId: 'auto' });
for (const capture of captures) {
  const validate = ajv.compile({ $ref: '#/components/schemas/' + capture.schema, components: { schemas } });
  assert(validate(capture.body), JSON.stringify({ schema: capture.schema, errors: validate.errors }));
}
console.log(`PASS ${captures.length} imported responses against native booking, ticket, PNR, pricing and report schemas.`);
