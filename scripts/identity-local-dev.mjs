#!/usr/bin/env node
// Resume only the explicitly configured localhost identity target. No import,
// migrations, provider request, or rollout activation is performed by startup.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawn, spawnSync } from 'node:child_process';
import { homedir } from 'node:os';
import { createHash } from 'node:crypto';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
assert(process.argv.length===2,'Usage: node scripts/identity-local-dev.mjs');
const file=path.join(root,'.local/identity-local/config.json');
const config=JSON.parse(fs.readFileSync(file,'utf8'));
assert((fs.statSync(file).mode&0o077)===0,'Identity config must be private (0600)');
const u=new URL(config.env.DATABASE_URL);
assert(['postgres:','postgresql:'].includes(u.protocol)&&u.hostname==='127.0.0.1'&&u.port==='55439'&&u.pathname.endsWith('_identity_test')&&!u.search&&!u.hash,'Selected localhost identity database required');
assert(config.env.PORTAL_IDENTITY_MODE==='canonical'&&config.env.APP_BIND==='127.0.0.1:18081','Pinned localhost canonical configuration required');
assert(config.env.PORTAL_IDENTITY_CLERK_SECRET_KEY?.startsWith('sk_test_'),'Clerk test instance required');
assert(!config.env.PORTAL_IDENTITY_OPERATOR_TOKEN&&!config.env.PORTAL_IDENTITY_OPERATOR_ID,'Operator credentials must not be retained in serving configuration');
const existing=await fetch('http://127.0.0.1:18081/health/live',{signal:AbortSignal.timeout(1500)}).then(r=>r.ok).catch(()=>false);
if (existing) {
  const ready=await fetch('http://127.0.0.1:18081/health/ready',{signal:AbortSignal.timeout(3000)}).then(r=>r.ok).catch(()=>false);
  console.log('Backend is already running at http://127.0.0.1:18081.');
  console.log(ready ? 'Health check passed. You can use the frontend; no second backend is needed.' : 'The process is running, but its readiness check failed. Check the running backend logs.');
  console.log('To rebuild and restart, stop its original launcher first, then run this command again.');
  process.exit(ready ? 0 : 1);
}
const pg=path.join(root,'.local/pgsql/bin');
if(spawnSync(path.join(pg,'pg_isready'),['-h','127.0.0.1','-p','55439'],{stdio:'ignore'}).status!==0) {
  const started=spawnSync(path.join(pg,'pg_ctl'),['-D',path.join(root,'.local/tier-check/data'),'-l',path.join(root,'.local/tier-check/postgres.log'),'-o','-h 127.0.0.1 -p 55439','-w','start'],{stdio:'inherit'});
  assert(started.status===0,'Existing localhost PostgreSQL could not start');
}
const built=spawnSync(path.join(homedir(),'.cargo/bin/cargo'),['build','--locked','--example','local_identity'],{cwd:root,stdio:'inherit'});
assert(built.status===0,'Local identity build failed');
// The running Rust adapter caches authentication. Restart on .env changes so
// changing supplier accounts/endpoints cannot keep an old login in memory.
const supplierFile=path.join(root,'.env');
const fingerprint=()=>{
  try { return createHash('sha256').update(fs.readFileSync(supplierFile)).digest('hex'); }
  catch { return 'unavailable'; }
};
let loaded=fingerprint();
let child;
let restartRequested=false;
let stopping=false;
let debounce;
function startBackend() {
  restartRequested=false;
  child=spawn(path.join(root,'target/debug/examples/local_identity'),[],{cwd:root,env:{PATH:process.env.PATH,...config.env},stdio:'inherit'});
  child.on('error',()=>{console.error('Could not start the local identity backend.');});
  child.on('exit',code=>{
    child=undefined;
    if (stopping) return;
    if (restartRequested) startBackend();
    else console.error(`Local backend stopped (${code ?? 'signal'}). Correct .env and save to retry, or restart the launcher.`);
  });
}
fs.watchFile(supplierFile,{interval:1000},()=>{
  clearTimeout(debounce);
  debounce=setTimeout(()=>{
    const next=fingerprint();
    if (stopping || next===loaded) return;
    loaded=next;
    console.log('Supplier .env changed; reloading supplier connections and capability flags. Run a fresh search.');
    restartRequested=true;
    if (child) child.kill('SIGTERM');
    else startBackend();
  },500);
});
for(const signal of ['SIGINT','SIGTERM'])process.on(signal,()=>{
  stopping=true;
  clearTimeout(debounce);
  fs.unwatchFile(supplierFile);
  if (child) child.kill(signal);
});
console.log('Supplier reads use .env (UAT or production); saves reload automatically. All suppliers use main-server booking/ticketing flags, database controls and shared adapter restrictions.');
startBackend();
