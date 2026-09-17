#!/usr/bin/env python3
"""Back up ONLY a disposable loopback identity fixture and restore to a NEW DB.
Never accepts a live host, drops a database, restores in place, or activates identity.
"""
import argparse, datetime, json, pathlib, re, subprocess
from urllib.parse import urlparse, urlunparse
p=argparse.ArgumentParser(description=__doc__)
p.add_argument('--source',required=True);p.add_argument('--target',required=True)
p.add_argument('--pg-bin',type=pathlib.Path,required=True);p.add_argument('--output',type=pathlib.Path,required=True)
a=p.parse_args();u=urlparse(a.source)
if u.scheme not in ('postgres','postgresql') or u.hostname not in ('127.0.0.1','localhost') or not re.fullmatch(r'/[a-z0-9_]+_identity_test',u.path) or u.query or u.fragment:
 p.error('source must be a disposable loopback database ending _identity_test')
if not re.fullmatch(r'identity_restore_[a-z0-9_]+_identity_test',a.target) or a.target==u.path[1:]:p.error('target must be a new identity_restore_*_identity_test database')
def run(program,*args):
 result=subprocess.run([str(a.pg_bin/program),*args],capture_output=True,text=True)
 if result.returncode:raise RuntimeError(program+' failed; no source changes were made')
 return result.stdout.strip()
def scalar(db,sql):return run('psql','-X','-A','-t','-v','ON_ERROR_STOP=1','--dbname',db,'-c',sql)
def snapshots(db):
 tables=scalar(db,"SELECT tablename FROM pg_tables WHERE schemaname='public' ORDER BY tablename").splitlines();out={}
 for t in tables:
  assert re.fullmatch('[a-z0-9_]+',t)
  out[t]=scalar(db,"SELECT count(*)::text||':'||md5(coalesce(string_agg(to_jsonb(t)::text,E'\\n' ORDER BY to_jsonb(t)::text),'')) FROM "+t+' t')
 seq=scalar(db,"SELECT coalesce(jsonb_agg(jsonb_build_array(sequencename,start_value,min_value,max_value,increment_by,cycle,last_value) ORDER BY sequencename),'[]'::jsonb) FROM pg_sequences WHERE schemaname='public'")
 return {'tables':out,'sequences':json.loads(seq)}
source_name=u.path[1:];target_url=urlunparse(u._replace(path='/'+a.target));admin_url=urlunparse(u._replace(path='/postgres'))
if scalar(admin_url,"SELECT count(*) FROM pg_database WHERE datname='"+a.target+"'")!='0':p.error('target exists; choose a NEW target (no overwrite supported)')
a.output.mkdir(parents=True,exist_ok=True);backup=a.output/(a.target+'.dump')
if backup.exists():p.error('backup path exists; choose a new output/target')
before=snapshots(a.source)
run('pg_dump','--format=custom','--no-owner','--no-acl','--file',str(backup),'--dbname',a.source)
scalar(admin_url,'CREATE DATABASE '+a.target)
run('pg_restore','--exit-on-error','--no-owner','--no-acl','--dbname',target_url,str(backup))
restored=snapshots(target_url);unchanged=snapshots(a.source)==before
if restored!=before or not unchanged:raise RuntimeError('restoration fingerprint mismatch; source and target retained for inspection')
evidence={'date':datetime.datetime.now(datetime.timezone.utc).isoformat(),'source_database':source_name,'restored_database':a.target,'source_unchanged':unchanged,'all_table_and_sequence_fingerprints_match':True,'tables':len(before['tables']),'schema_version':scalar(target_url,'SELECT max(version) FROM _sqlx_migrations'),'backup_path':str(backup.resolve()),'activation_performed':False}
(a.output/(a.target+'.json')).write_text(json.dumps(evidence,indent=2)+'\n');print(json.dumps(evidence,indent=2))
