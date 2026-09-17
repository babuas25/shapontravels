// Explicit staged operator command. No .env is automatically loaded.
// Usage: node scripts/identity-bootstrap.mjs user_SELECTED_CLERK_ID
const fail = message => { console.error(message); process.exitCode = 1; };
async function main() {
  const args = process.argv.slice(2);
  if (args.length !== 1 || !/^user_[A-Za-z0-9_]{1,123}$/.test(args[0])) throw new Error('Usage: node scripts/identity-bootstrap.mjs user_SELECTED_CLERK_ID');
  if (process.env.PORTAL_IDENTITY_MODE !== 'staged') throw new Error('Explicit staged mode is required. Live activation is not implemented.');
  let url;
  try { url = new URL(process.env.SHAPON_API_BASE_URL); } catch { throw new Error('Set SHAPON_API_BASE_URL to the isolated Rust service.'); }
  if (!['127.0.0.1', 'localhost'].includes(url.hostname) || url.protocol !== 'http:' || url.pathname !== '/' || url.username || url.password || url.search || url.hash) {
    throw new Error('This staged bootstrap command only permits a loopback Rust service.');
  }
  const token = process.env.PORTAL_IDENTITY_OPERATOR_TOKEN;
  if (!/^stio_[A-Za-z0-9_-]{43}$/.test(token ?? '')) throw new Error('A dedicated operator credential is required.');
  let response;
  try {
    response = await fetch(`${url.origin}/admin/portal-identity/bootstrap`, {
      method: 'POST', redirect: 'error', signal: AbortSignal.timeout(18000),
      headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' },
      body: JSON.stringify({ clerk_user_id: args[0] }),
    });
  } catch { throw new Error('Bootstrap result unconfirmed. Inspect the staged session/registry before retrying.'); }
  const body = await response.json().catch(() => null);
  if (!response.ok) throw new Error(typeof body?.error === 'string' && /^IDENTITY_[A-Z_]{1,80}$/.test(body.error) ? body.error : 'Identity bootstrap failed.');
  if (body?.authority_mode !== 'staged' || body?.user?.clerk_user_id !== args[0] || body?.user?.role !== 'superadmin') throw new Error('Unexpected response. Inspect the registry before retrying.');
  console.log(`Staged bootstrap completed for ${args[0]}. Live portal authority is unchanged. Remove the operator credential from the Rust service configuration.`);
}
main().catch(error => fail(error.message));
