// Two-step operator tool. `plan` has no network; `apply` is explicit and never retries.
// No dotenv, credential persistence, data import/deletion or automatic rollback.
import { createHash } from 'node:crypto';
import { open, readFile, stat, writeFile } from 'node:fs/promises';
import { pathToFileURL } from 'node:url';
import { validateReport } from './identity-preflight.mjs';
const sha = value => createHash('sha256').update(value).digest('hex');
const digest = value => sha(JSON.stringify(value));
const hash = value => typeof value==='string' && /^[a-f0-9]{64}$/.test(value);
const uuid = value => typeof value==='string' && /^[a-f0-9]{8}-[a-f0-9]{4}-[1-5][a-f0-9]{3}-[89ab][a-f0-9]{3}-[a-f0-9]{12}$/.test(value);
const fail = message => {throw new Error(message);};
async function bytes(file) { try {if((await stat(file)).size>1048576)throw Error();return await readFile(file);}catch{fail('Review/evidence file missing or exceeds 1 MiB.');} }
async function json(file) {try{return JSON.parse((await bytes(file)).toString('utf8'));}catch{fail('Invalid review/plan JSON file.');} }
function origin(value) {try{const u=new URL(value);if((u.protocol!=='https:'&&!(u.protocol==='http:'&&['localhost','127.0.0.1'].includes(u.hostname)))||u.pathname!=='/'||u.username||u.password||u.search||u.hash)throw Error();return u.origin;}catch{fail('Target must be an explicit HTTPS or loopback HTTP origin.');}}
function pinValid(p) {return p&&uuid(p.id)&&/^[a-z0-9][a-z0-9_-]{0,63}$/.test(p.target_id)&&Number.isSafeInteger(p.revision)&&p.revision>0&&hash(p.backend_release)&&hash(p.frontend_release)&&Object.keys(p).length===5;}
function validatePlan(p) {
  if(!p||p.format_version!==1||!pinValid(p.pin)||!Number.isFinite(Date.parse(p.created_at))||Math.abs(Date.now()-Date.parse(p.created_at))>900000||origin(p.target_origin)!==p.target_origin)fail('Invalid or expired rollout plan.');
  const c=p.command;
  if(!c||!/^user_[A-Za-z0-9_]{1,123}$/.test(c.clerk_user_id)||c.rollout_id!==p.pin.id||!Number.isSafeInteger(c.expected_revision)||c.expected_revision<0||!['activate','pause','resume'].includes(c.action)||Object.keys(c).length!==5)fail('Invalid rollout command.');
  if(c.action==='activate'&&(c.expected_revision!==0||p.pin.revision!==1))fail('Initial activation requires revision 1.');
  if(c.action==='resume'&&p.pin.revision!==c.expected_revision+1)fail('Resume requires the next revision.');
  if(c.action!=='pause') {
    const e=c.evidence;
    const evidenceKeys=['mapping_digest','backup_sha256','restore_sha256','writers_shutdown_sha256','configuration_sha256','acknowledge_activation','acknowledge_unresolved_recovery','acknowledge_retained_bookings'];
    if(!e||e.acknowledge_activation!==true||typeof e.acknowledge_unresolved_recovery!=='boolean'||![7,8].includes(Object.keys(e).length)||Object.keys(e).some(k=>!evidenceKeys.includes(k))||(Object.hasOwn(e,'acknowledge_retained_bookings')&&typeof e.acknowledge_retained_bookings!=='boolean')||!['mapping_digest','backup_sha256','restore_sha256','writers_shutdown_sha256','configuration_sha256'].every(k=>hash(e[k])))fail('Explicit review/backup/restore/shutdown/configuration evidence is required.');
    if(c.action==='activate'&&e.acknowledge_unresolved_recovery)fail('Initial activation cannot waive unresolved work.');
  } else if(c.evidence!==null)fail('Pause does not accept activation evidence.');
  return p;
}
async function plan(manifestPath,output) {
  const m=await json(manifestPath);
  const allowed=['format_version','target_origin','operator_subject','pin','expected_revision','action','review_file','backup_evidence_file','restore_evidence_file','writers_shutdown_evidence_file','configuration_evidence_file','acknowledge_activation','acknowledge_unresolved_recovery','acknowledge_retained_bookings'];
  if(!m||Object.keys(m).some(k=>!allowed.includes(k)))fail('Unknown manifest field.');
  const p={format_version:1,created_at:new Date().toISOString(),target_origin:origin(m.target_origin),pin:m.pin,command:{clerk_user_id:m.operator_subject,rollout_id:m.pin?.id,expected_revision:m.expected_revision,action:m.action,evidence:null}};
  if(m.action!=='pause') {
    const file=await json(m.review_file);
    if(!file?.review||file.sha256!==digest(file.review)||file.review.target_origin!==p.target_origin||file.review.operator_sha256!==sha(m.operator_subject)||file.review.mapping_drift===true)fail('Review belongs to a different target/operator or contains drift.');
    const r=validateReport(file.review.report);
    const retainBookings=m.action==='activate'&&m.acknowledge_retained_bookings===true&&r.backlog.bookings.pending>0&&Object.entries(r.backlog).every(([name,work])=>name==='bookings'||work.pending===0);
    if(!r.maintenance_enabled||!r.worker_paused||!r.bootstrap_ready||!r.selected_operator_active_superadmin||Object.values(r.mapping_issues).some(n=>n!==0)||(!r.database_review_clear&&!retainBookings&&!(m.action==='resume'&&m.acknowledge_unresolved_recovery===true)))fail('Database review has unresolved gates.');
    const e={mapping_digest:r.mapping_digest};
    for(const key of ['backup','restore','writers_shutdown','configuration']) {const b=await bytes(m[`${key}_evidence_file`]);if(!b.length)fail('Evidence files cannot be empty.');e[`${key}_sha256`]=sha(b);}
    e.acknowledge_activation=m.acknowledge_activation;e.acknowledge_unresolved_recovery=m.acknowledge_unresolved_recovery??false;
    if(Object.hasOwn(m,'acknowledge_retained_bookings'))e.acknowledge_retained_bookings=m.acknowledge_retained_bookings;
    p.command.evidence=e;
  }
  validatePlan(p);
  try{await writeFile(output,JSON.stringify({plan:p,sha256:digest(p)},null,2)+'\n',{flag:'wx',mode:0o600});}catch{fail('Plan output must be a new private file.');}
  console.log(JSON.stringify({plan_saved:true,action:p.command.action,network_requests:0,activation_performed:false}));
  return 0;
}
async function request(target,route,token,body) {
  const r=await fetch(`${target}/admin/portal-identity/${route}`,{method:'POST',redirect:'error',cache:'no-store',signal:AbortSignal.timeout(20000),headers:{Authorization:`Bearer ${token}`,'Content-Type':'application/json'},body:JSON.stringify(body)});
  if(!r.body)throw Error();const reader=r.body.getReader(),chunks=[];let size=0;
  try{for(;;){const {value,done}=await reader.read();if(done)break;size+=value.length;if(size>65536){await reader.cancel();throw Error();}chunks.push(value);}}finally{reader.releaseLock();}
  const value=JSON.parse(Buffer.concat(chunks).toString('utf8'));if(!r.ok)throw Error();return value;
}
async function apply(planPath,output) {
  const file=await json(planPath);if(!file?.plan||file.sha256!==digest(file.plan))fail('Plan digest does not match.');const p=validatePlan(file.plan);
  const operator=process.env.PORTAL_IDENTITY_OPERATOR_TOKEN,bridge=process.env.PORTAL_IDENTITY_BRIDGE_TOKEN;
  if(!/^stio_[A-Za-z0-9_-]{43}$/.test(operator??'')||!/^stib_[A-Za-z0-9_-]{43}$/.test(bridge??''))fail('Independent operator and bridge credentials are required.');
  let out;try{out=await open(output,'wx',0o600);}catch{fail('Result output must be a new private file; no request was sent.');}
  let dispatched=false;
  async function record(value) {await out.truncate(0);await out.write(JSON.stringify(value,null,2)+'\n',0,'utf8');await out.sync();}
  try {
    await record({plan_sha256:file.sha256,outcome:'not_started',activation_performed:false});
    const ready=await request(p.target_origin,'readiness',bridge,{});
    if(ready.authority_mode!=='canonical'||ready.maintenance_enabled!==true||ready.worker_paused!==true||ready.schema_ready!==true||ready.bootstrap_ready!==true)throw Error();
    for(const [key,value]of Object.entries(p.pin))if(ready.runtime_pin?.[key]!==value)throw Error();
    const c=p.command;
    if(c.action==='activate'?ready.rollout!==null:(!ready.rollout||ready.rollout.id!==p.pin.id||ready.rollout.target_id!==p.pin.target_id||ready.rollout.revision!==c.expected_revision||ready.rollout.state!==(c.action==='pause'?'active':'paused')))throw Error();
    await record({plan_sha256:file.sha256,outcome:'dispatching',activation_performed:null});
    dispatched=true;
    const result=await request(p.target_origin,'rollout',operator,c);
    if(result.id!==p.pin.id||result.target_id!==p.pin.target_id||result.revision!==c.expected_revision+1||result.state!==(c.action==='pause'?'paused':'active'))throw Error();
    const releases=c.action==='pause'?ready.rollout:p.pin;
    if(result.backend_release!==releases.backend_release||result.frontend_release!==releases.frontend_release)throw Error();
    const marker=Object.fromEntries(['id','target_id','revision','state','backend_release','frontend_release'].map(k=>[k,result[k]]));
    await record({plan_sha256:file.sha256,outcome:'confirmed',marker,activation_performed:c.action!=='pause',maintenance_still_required:true});
    console.log(JSON.stringify({outcome:'confirmed',revision:marker.revision,state:marker.state,maintenance_still_required:true}));return 0;
  }catch{
    await record({plan_sha256:file.sha256,outcome:dispatched?'unconfirmed':'not_started',activation_performed:dispatched?null:false,inspect_marker_before_any_retry:true});
    console.error(dispatched?'Rollout result unconfirmed. Inspect the marker and retained result before any retry.':'Rollout readiness/configuration failed. No mutation request was sent.');return dispatched?2:1;
  }finally{await out.close();}
}
export async function main(args=process.argv.slice(2)) {
  const [mode,flag,input,outFlag,output,...extra]=args;
  if(extra.length||!input||!output||outFlag!=='--output'||!((mode==='plan'&&flag==='--manifest')||(mode==='apply'&&flag==='--plan')))fail('Usage: identity-rollout.mjs plan --manifest FILE --output NEW_FILE | apply --plan FILE --output NEW_FILE');
  return mode==='plan'?plan(input,output):apply(input,output);
}
if(process.argv[1]&&import.meta.url===pathToFileURL(process.argv[1]).href)main().then(code=>{process.exitCode=code;}).catch(()=>{console.error('Rollout configuration/artifact check failed. Existing files were preserved; no automatic retry occurs.');process.exitCode=1;});
