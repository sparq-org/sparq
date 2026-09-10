import pathlib,subprocess,os,time,json,signal,sys
p=pathlib.Path(__file__).resolve().parent
w=pathlib.Path('/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue6480')
node='/Users/jesght/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin/node'
cli='/private/tmp/sparq-pr6049/.throughput-monitor/direct-6480/implementation/tools/npm-11.17.0/bin/npm-cli.js'
label=sys.argv[1]
args=sys.argv[2:]
def resource():
 v=os.statvfs(p)
 return {'free_bytes':v.f_bavail*v.f_frsize,'allocated_bytes':sum(x.stat().st_blocks*512 for x in p.rglob('*') if x.is_file())}
def allowed(r):return r['free_bytes']>=2*1024**3 and r['allocated_bytes']<=256*1024**2
env={'PATH':str(pathlib.Path(node).parent)+':/usr/bin:/bin:/opt/homebrew/bin','TMPDIR':str(p/'tmp'),'LANG':'en_US.UTF-8','npm_config_cache':str(p/'cache'),'npm_config_userconfig':str(p/'empty-user.npmrc'),'npm_config_globalconfig':str(p/'empty-global.npmrc'),'npm_config_registry':'https://registry.npmjs.org','npm_config_fetch_retries':'0','npm_config_fetch_timeout':'30000','npm_config_update_notifier':'false','npm_config_ignore_scripts':'true','npm_config_audit':'false','npm_config_fund':'false'}
start=resource();assert allowed(start),start
t=time.monotonic();samples=[start];cmd=[node,cli]+args
with (p/(label+'.log')).open('w') as f:
 child=subprocess.Popen(cmd,cwd=w,env=env,stdout=f,stderr=subprocess.STDOUT,start_new_session=True)
 stop=None
 while child.poll() is None:
  r=resource();samples.append(r)
  if not allowed(r) or time.monotonic()-t>180:
   stop='resource-or-180s-time-limit';os.killpg(child.pid,signal.SIGTERM);break
  if len(samples)%10==0:print(label,round(time.monotonic()-t,1),r,flush=True)
  time.sleep(1)
 try:rc=child.wait(timeout=10)
 except subprocess.TimeoutExpired:os.killpg(child.pid,signal.SIGKILL);rc=child.wait()
last=resource();out={'argv':cmd,'env':env,'exit_code':rc,'stop':stop,'elapsed_seconds':time.monotonic()-t,'initial':start,'final':last,'max_allocated_bytes':max(x['allocated_bytes'] for x in samples+[last]),'min_free_bytes':min(x['free_bytes'] for x in samples+[last])}
(p/(label+'.json')).write_text(json.dumps(out,indent=2)+'\n');print(json.dumps(out,indent=2));sys.exit(rc or (1 if stop else 0))
