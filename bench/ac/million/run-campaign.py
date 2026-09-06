#!/usr/bin/env python3
"""[GPT-6] Execute a frozen, bounded Linux campaign with separate server cgroups."""
import argparse
import datetime
import hashlib
import itertools
import json
import math
import os
from pathlib import Path
import shutil
import subprocess
import time
import urllib.error
import urllib.request


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def sha(path):
    digest = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            digest.update(block)
    return digest.hexdigest()


def tail(path,limit=4096):
    if not path.exists():return ''
    with path.open('rb') as stream:
        stream.seek(max(0,path.stat().st_size-limit))
        return stream.read().decode('utf-8',errors='replace')


def compress(path):
    if path.exists():
        subprocess.run(['zstd', '-q', '-f', '--rm', str(path)], check=True)


def summary(path, deadline):
    counts = {}; durations = []; timely = 0; offered = 0; dropped = 0; bad_receipts = 0; lagged = 0
    queue_samples = []; scheduled_durations = []; sequences=set(); completion=None
    bad_plan_counts=0; modification_noops=0
    policy_attempts = policy_changed = policy_noop = 0
    inserted = deleted = 0
    with path.open() as stream:
        for line in stream:
            row = json.loads(line)
            if row['record_type'] == 'load-complete': completion=row
            if row['record_type'] != 'request':
                continue
            sequences.add(row['sequence'])
            offered += 1
            outcome = row.get('outcome', 'missing')
            counts[outcome] = counts.get(outcome, 0) + 1
            if outcome == 'client-admission-drop': dropped += 1
            if row.get('dispatch_lag_us', 0) > deadline * 1000: lagged += 1
            if row.get('http_latency_us') is not None: durations.append(row['http_latency_us'])
            if row.get('scheduled_latency_us') is not None: scheduled_durations.append(row['scheduled_latency_us'])
            if row.get('queue_us') is not None: queue_samples.append((row['sequence'],row['queue_us']))
            if outcome == 'ok' and row.get('scheduled_latency_us', math.inf) <= deadline * 1000: timely += 1
            receipt = row.get('mutation_receipt') or {}
            inserted += receipt.get('inserted_records', 0); deleted += receipt.get('deleted_records', 0)
            if row.get('operation') == 'policy-attempt':
                policy_attempts += 1
                if outcome == 'ok':
                    delta = row.get('policy_triple_delta', 0)
                    policy_changed += int(delta != 0); policy_noop += int(delta == 0)
            # An absent/deleted selected modification can legitimately be a no-op.
            # The durable audit checks exact replayed state; missing receipts are failures.
            if row.get('planned_records') is not None and outcome == 'ok':
                if not row.get('mutation_receipt_present', False): bad_receipts += 1
                if not row.get('mutation_matches_plan',False):
                    allowed_noop=row.get('operation')=='modify' and receipt.get('inserted_triples')==0 and receipt.get('deleted_triples')==0 and all(v==0 for v in receipt.get('poststate_triples',[1]))
                    if allowed_noop: modification_noops+=1
                    else: bad_plan_counts+=1
    durations.sort(); scheduled_durations.sort(); queue_samples.sort()
    def p95(values):
        values=sorted(values)
        return values[max(0,math.ceil(.95*len(values))-1)] if values else None
    width=max(1,offered//4)
    first=p95([v for sequence,v in queue_samples if sequence<width])
    last=p95([v for sequence,v in queue_samples if sequence>=offered-width])
    return {'offered': offered, 'scheduled_coverage_complete':bool(completion and completion['offered']==offered==len(sequences) and sequences==set(range(offered))), 'unexpected_mutation_count_mismatches':bad_plan_counts, 'acknowledged_absent_record_modification_noops':modification_noops, 'outcomes': counts, 'successful': counts.get('ok', 0),
            'within_local_deadline': timely, 'client_dropped': dropped, 'dispatch_lag_over_deadline': lagged,
            'p95_http_us': p95(durations), 'p95_scheduled_us':p95(scheduled_durations),
            'queue_timer_coverage':len(queue_samples)/offered if offered else 0,
            'first_quarter_p95_queue_us':first,'last_quarter_p95_queue_us':last,
            'acknowledged_inserted_records': inserted, 'acknowledged_deleted_records': deleted,
            'acknowledged_net_records': inserted-deleted, 'missing_mutation_receipts': bad_receipts,
            'policy_attempts': policy_attempts, 'acknowledged_effective_policy_changes': policy_changed,
            'acknowledged_policy_noops': policy_noop,
            'unknown_commits': counts.get('transport-error', 0)+counts.get('body-error', 0)}


def representatives(path):
    classes={}
    with path.open() as stream:
        for line in stream:
            row=json.loads(line)
            key=(row['intensity_numerator'],row['intensity_denominator'])
            candidate={'pod':row['pod_id'],'source_bytes':row['bytes'],'intensity_numerator':key[0],'intensity_denominator':key[1]}
            if key not in classes:classes[key]=[candidate,candidate]
            if (candidate['source_bytes'],candidate['pod'])<(classes[key][0]['source_bytes'],classes[key][0]['pod']):classes[key][0]=candidate
            if (candidate['source_bytes'],-candidate['pod'])>(classes[key][1]['source_bytes'],-classes[key][1]['pod']):classes[key][1]=candidate
    unique={row['pod']:row for pair in classes.values() for row in pair}
    return [unique[pod] for pod in sorted(unique)]


def reconcile(raw, audit):
    """Join complete client acknowledgements to exactly one durable receipt."""
    committed={}; errors=[]; resolved=[]; offered=set(); acknowledged=0
    with audit.open() as stream:
        for line in stream:
            row=json.loads(line)
            if row.get('record_type')!='committed-mutation':continue
            receipt=row['receipt']; identity=receipt['id']
            if identity in committed:errors.append({'id':identity,'error':'duplicate-durable-receipt'})
            committed[identity]=(row['pod'],receipt)
    with raw.open() as stream:
        for line in stream:
            row=json.loads(line)
            if row.get('record_type')!='request' or 'mutation_id' not in row:continue
            identity=row['mutation_id']
            if identity in offered:errors.append({'id':identity,'error':'duplicate-offered-mutation-id'})
            offered.add(identity)
            durable=committed.get(identity)
            if row.get('outcome')=='ok':
                acknowledged+=1
                if not row.get('mutation_receipt') or durable!=(row['pod'],row['mutation_receipt']):
                    errors.append({'id':identity,'error':'acknowledgement-not-identically-durable'})
            elif row.get('outcome') in ['transport-error','body-error']:
                resolved.append({'id':identity,'committed':durable is not None})
            elif durable is not None:
                errors.append({'id':identity,'error':'failed-or-unsent-request-committed'})
    for identity in committed.keys()-offered:
        errors.append({'id':identity,'error':'durable-mutation-without-offered-request'})
    return {'passed':not errors,'acknowledged':acknowledged,'durable_receipts':len(committed),
            'unknown_outcomes_resolved':resolved,'errors':errors}


class Campaign:
    def __init__(self, args):
        self.root = Path(__file__).resolve().parents[3]
        self.spec = json.loads(args.campaign.read_text())
        self.results = args.results; self.results.mkdir(parents=True, exist_ok=True)
        self.corpora = args.corpora; self.corpora.mkdir(parents=True, exist_ok=True)
        self.binary = args.binary.resolve(); self.auth = args.auth.resolve()
        self.started = time.monotonic(); self.current_server = None
        timestamp=self.results/"started-at.txt"
        if timestamp.exists():
            origin=datetime.datetime.fromisoformat(timestamp.read_text().strip().replace("Z","+00:00")).timestamp()
            self.started-=max(0,time.time()-origin)
        self.limit = self.spec['stop_rules']['runtime_ceiling_seconds']
        self.floor = self.spec['stop_rules']['disk_floor_bytes']
        self.measurement = self.spec['measurement']
        self.client_cpus = ','.join(map(str, self.spec['host']['client_cpu_pool']))
        self.counter = 0
        shutil.copyfile(args.campaign, self.results / 'campaign.json')
        write_json(self.results / 'campaign-runtime.json', {'binary_sha256': sha(self.binary), 'started_unix': time.time(),
                   'scope': 'Local complete HTTP body guard; network journeys unmeasured; full-history million scale conditional and unmeasured unless reached',
                   'parent_slice': 'sparq-pod-bench.slice', 'client_cpu_pool': self.client_cpus})

    def event(self, value):
        with (self.results / 'campaign-events.jsonl').open('a') as stream:
            stream.write(json.dumps(value) + '\n')

    def guard(self, estimate=0):
        elapsed = time.monotonic()-self.started
        free = shutil.disk_usage(self.corpora).free
        if elapsed + estimate > self.limit: raise RuntimeError('frozen campaign runtime ceiling')
        if free < self.floor: raise RuntimeError('frozen disk floor')

    def run(self, command, log, timeout=None):
        self.guard()
        with log.open('w') as stream:
            process = subprocess.Popen(command, cwd=self.root, stdout=stream, stderr=subprocess.STDOUT)
            start = time.monotonic()
            try:
                while process.poll() is None:
                    self.guard()
                    if timeout is not None and time.monotonic()-start > timeout: raise TimeoutError(str(command[:3]))
                    time.sleep(.5)
            except BaseException:
                process.terminate()
                try: process.wait(timeout=10)
                except subprocess.TimeoutExpired: process.kill(); process.wait()
                raise
            return process.returncode

    def cgroup(self, unit):
        path = subprocess.check_output(['sudo','systemctl','show',unit,'--property=ControlGroup','--value'],text=True).strip()
        if not path or not path.startswith('/sparq-pod-bench.slice/') or not path.endswith('/'+unit):
            raise OSError('server cgroup path unavailable or outside study slice')
        return Path('/sys/fs/cgroup') / path.lstrip('/')

    def resource(self, unit, label):
        result = {}
        try:
            group = self.cgroup(unit)
            for name in ['memory.current','memory.peak','memory.events','memory.stat','cpu.stat','io.stat','pids.current']:
                if (group/name).exists(): result[name]=(group/name).read_text()
            pid = subprocess.check_output(['sudo','systemctl','show',unit,'--property=MainPID','--value'],text=True).strip()
            if pid != '0':
                for name in ['status','io','smaps_rollup']:
                    result['proc/'+name] = subprocess.check_output(['sudo','cat',f'/proc/{pid}/{name}'],text=True,stderr=subprocess.DEVNULL)
        except (OSError,subprocess.CalledProcessError) as error: result['error']=str(error)
        write_json(self.results / f'{label}-resources.json', result)

    def stop_server(self):
        if self.current_server:
            subprocess.run(['sudo','systemctl','stop',self.current_server],check=False,stdout=subprocess.DEVNULL)
            subprocess.run(['sudo','systemctl','reset-failed',self.current_server],check=False,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
            self.current_server=None

    def start_server(self, corpus, group, memory, cpus, label):
        self.counter += 1
        unit=f'sparq-pod-server-{os.getpid()}-{self.counter}.service'
        self.current_server=unit
        command=['sudo','systemd-run','--quiet',f'--unit={unit}','--slice=sparq-pod-bench.slice',
            f'--uid={os.getuid()}', f'--gid={os.getgid()}',f'--working-directory={self.root}',
            '--property=MemoryAccounting=yes','--property=CPUAccounting=yes','--property=IOAccounting=yes',
            f'--property=MemoryMax={memory*1024**3}','--property=MemorySwapMax=0',
            f'--property=StandardOutput=append:{self.results / (label+"-server.log")}',
            f'--property=StandardError=append:{self.results / (label+"-server.log")}',
            'taskset','-c',','.join(map(str,self.spec['host']['server_cpu_pool'][:cpus])),str(self.binary),'serve',
            '--corpus',str(corpus),'--auth-dir',str(self.auth),'--workers',str(cpus),
            '--cache-pods',str(group['cache_pods']),'--cache-bytes',str(group['cache_source_bytes']),
            '--max-pod-bytes',str(group['max_pod_bytes']),
            '--queue-capacity',str(self.measurement['queue_capacity_per_worker'])]
        subprocess.run(command,check=True)
        for _ in range(100):
            try:
                with urllib.request.urlopen('http://127.0.0.1:3100/', timeout=.2): return unit
            except urllib.error.HTTPError: return unit
            except urllib.error.URLError: time.sleep(.1)
        raise RuntimeError('server startup failed')

    def archive_journals(self,corpus,label):
        if (corpus/'updates').exists():
            subprocess.run(['tar','--zstd','-cf',str(self.results/f'{label}-quarantined-journals.tar.zst'),'-C',str(corpus),'updates'],check=True)

    def cell(self, dataset, corpus, model, group, memory, cpus, rate, replicate):
        seed=self.spec['seeds'][replicate]
        label=f'{dataset["id"]}-{group["id"]}-ram{memory}-cpu{cpus}-r{rate}-{replicate}-{model}'
        (self.results/'stage.txt').write_text(label+'\n')
        if (corpus/'updates').exists(): shutil.rmtree(corpus/'updates')
        base=['taskset','-c',self.client_cpus,str(self.binary),'load','--corpus',str(corpus),'--auth-dir',str(self.auth),
              '--mix','journeys','--workload-file',str(self.root/'bench/ac/million/workload.json'),
              '--scenario',group['scenario'],'--selection',group['selection'],'--seed',str(seed),
              '--timeout-ms',str(self.measurement['request_timeout_ms']),
              '--max-inflight',str(self.measurement['maximum_inflight'])]
        if rate != 'derived': base += ['--rate',str(rate)]
        plan=self.results/f'{label}-plan.jsonl'
        if self.run(base+['--plan-only','true','--out',str(plan)],self.results/f'{label}-plan.log'):
            raise RuntimeError('workload planning failed')
        offered_rate=json.loads(plan.read_text().splitlines()[0])['offered_rate']
        duration=max(self.measurement['measurement_seconds'],self.measurement['minimum_offered']/offered_rate)
        self.guard(duration+self.measurement['warmup_seconds']+60)
        # Cold origin: explicit OS drop before process start. Warmup itself is recorded.
        subprocess.run(['sudo','sh','-c','sync; echo 3 > /proc/sys/vm/drop_caches'],check=True)
        unit=self.start_server(corpus,group,memory,cpus,label)
        raw=self.results/f'{label}-requests.jsonl'
        try:
            warm_requests=max(1,math.ceil(self.measurement['warmup_seconds']*offered_rate)+1)
            warm=['taskset','-c',self.client_cpus,str(self.binary),'load','--corpus',str(corpus),'--auth-dir',str(self.auth),
                  '--mix','reads','--pods',str(dataset['pods']),'--query-set','population','--principal','owner','--selection',group['selection'],
                  '--arrival','constant','--seed',str(seed),'--rate',str(offered_rate),'--requests',str(warm_requests),
                  '--timeout-ms',str(self.measurement['request_timeout_ms']),'--max-inflight',str(self.measurement['maximum_inflight']),
                  '--out',str(self.results/f'{label}-warmup.jsonl')]
            warm_status=self.run(warm,self.results/f'{label}-warmup.log',timeout=self.measurement['warmup_seconds']+60)
            self.resource(unit,label+'-before')
            status=self.run(base+['--duration-seconds',str(self.measurement['measurement_seconds']),
                                 '--requests',str(self.measurement['minimum_offered']), '--out',str(raw)],
                            self.results/f'{label}-load.log',timeout=duration+120)
            self.resource(unit,label+'-after')
            result=summary(raw,self.measurement['server_deadline_ms']) if raw.exists() else {'offered':0}
            result.update(label=label,dataset=dataset["id"],group=group["id"],model=model,memory_gib=memory,cpus=cpus,rate_override=rate,replicate=replicate,seed=seed,load_exit_code=status,warmup_exit_code=warm_status,offered_rate=offered_rate)
        finally: self.stop_server()
        audit=self.results/f'{label}-audit.jsonl'
        audit_status=self.run([str(self.binary),'audit','--corpus',str(corpus),'--cache-pods','1',
                              '--cache-bytes',str(group['cache_source_bytes']),'--max-pod-bytes',str(group['max_pod_bytes'])],audit)
        result['audit_exit_code']=audit_status
        reconciliation=reconcile(raw,audit) if audit_status==0 and raw.exists() else {'passed':False,'error':'audit or client record unavailable'}
        write_json(self.results/f'{label}-reconciliation.json',reconciliation)
        result['acknowledgements_reconciled']=reconciliation['passed']
        definite_audit_mismatch='replayed record mismatch' in tail(audit)
        definite_correctness_failure=definite_audit_mismatch or (audit_status==0 and not reconciliation['passed']) or result.get('missing_mutation_receipts',0)>0 or result.get('unexpected_mutation_count_mismatches',0)>0
        result['client_limited']=result.get('client_dropped',0)>0 or result.get('dispatch_lag_over_deadline',0)>0
        result['correctness_passed']=reconciliation['passed'] and audit_status==0 and result.get('missing_mutation_receipts',0)==0 and result.get('unexpected_mutation_count_mismatches',0)==0
        resource_checks=[]
        for phase in ['before','after']:
            resources=json.loads((self.results/f'{label}-{phase}-resources.json').read_text())
            events={line.split()[0]:int(line.split()[1]) for line in resources.get('memory.events','').splitlines()}
            peak=int(resources['memory.peak'].strip()) if resources.get('memory.peak','').strip().isdigit() else None
            resource_checks.append(bool('error' not in resources and peak is not None and peak<=memory*1024**3 and 'oom' in events and events['oom']==0 and events.get('oom_kill',0)==0))
        result['resource_guard_passed']=all(resource_checks)
        first=result.get('first_quarter_p95_queue_us'); last=result.get('last_quarter_p95_queue_us')
        stability=self.measurement['queue_stability']
        tolerance=max(stability['allowed_growth_us_floor'],(first or 0)*stability['allowed_growth_fraction_of_first_quarter'])
        result['queue_growth_tolerance_us']=tolerance
        result['queue_stability_passed']=bool(first is not None and last is not None and last<=first+tolerance and result.get('queue_timer_coverage',0)>=stability['required_timer_coverage_fraction_of_offered'])
        result['passes_local_guard']=bool(result.get('scheduled_coverage_complete',False) and result['correctness_passed'] and result['resource_guard_passed'] and result['queue_stability_passed'] and status==0 and not result['client_limited'] and result['offered']>0 and
            result.get('successful',0)/result['offered']>=self.measurement['success_fraction'] and
            result.get('within_local_deadline',0)/result['offered']>=self.measurement['deadline_fraction_of_all_offered'])
        result['capacity_interpretation']='inconclusive-client-limited' if result['client_limited'] else 'tested local guard only; full claim requires all declared operations/cells'
        write_json(self.results/f'{label}-summary.json',result)
        for path in [raw,audit,self.results/f'{label}-warmup.jsonl']: compress(path)
        self.event({'record_type':'cell-complete',**result})
        if not result['correctness_passed']:
            self.archive_journals(corpus,label)
            if definite_correctness_failure:
                self.event({'record_type':'correctness-quarantine','label':label,'dataset':dataset['id'],'model':model,'reason':'wrong state, receipt or mutation count','retained_journals':str(corpus/'updates')})
                raise RuntimeError('post-run correctness failure; journals preserved and source quarantined')
            result['dataset_quarantined']=True
            self.event({'record_type':'dataset-not-capacity-eligible','dataset':dataset['id'],'model':model,'label':label,'reason':'mutation audit unavailable; resource/error evidence retained','restriction':'Unverified journal replay prevents capacity/equivalence admission; no incorrect result inferred.'})
        return result

    def verify_representative(self,dataset,corpus,model,representative):
        self.counter+=1;pod=representative['pod']
        label=f'{dataset["id"]}-{model}-verify-pod{pod}'
        unit=f'sparq-pod-verify-{os.getpid()}-{self.counter}.service'
        log=self.results/f'{label}.jsonl'
        command=['sudo','systemd-run','--quiet',f'--unit={unit}','--slice=sparq-pod-bench.slice',
                 f'--uid={os.getuid()}',f'--gid={os.getgid()}',f'--working-directory={self.root}',
                 '--property=RemainAfterExit=yes','--property=MemoryAccounting=yes','--property=CPUAccounting=yes','--property=IOAccounting=yes',
                 '--property=MemoryMax=17179869184','--property=MemorySwapMax=0',
                 f'--property=StandardOutput=append:{log}',f'--property=StandardError=append:{log}',
                 str(self.binary),'verify','--corpus',str(corpus),'--verify-pod-ids',str(pod),
                 '--cache-pods','1','--cache-bytes',str(2*1024**3),'--max-pod-bytes',str(2*1024**3),
                 '--workload-file',str(self.root/'bench/ac/million/workload.json')]
        subprocess.run(command,check=True)
        try:
            while True:
                self.guard()
                state=subprocess.check_output(['sudo','systemctl','show',unit,'--property=SubState','--value'],text=True).strip()
                if state in ['exited','failed','dead']:break
                time.sleep(.5)
            status=int(subprocess.check_output(['sudo','systemctl','show',unit,'--property=ExecMainStatus','--value'],text=True).strip())
            reason=subprocess.check_output(['sudo','systemctl','show',unit,'--property=Result','--value'],text=True).strip()
            self.resource(unit,label)
        finally:
            subprocess.run(['sudo','systemctl','stop',unit],check=False,stdout=subprocess.DEVNULL)
            subprocess.run(['sudo','systemctl','reset-failed',unit],check=False,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        log_tail=tail(log)
        classification=('verified' if status==0 and reason=='success' else 'verification-oom' if reason=='oom-kill' else
                        'source-admission-failed' if 'exceeds configured admission budget' in log_tail else
                        'oracle-mismatch' if 'mismatch' in log_tail else 'verification-error')
        self.event({'record_type':'representative-verification','dataset':dataset['id'],'model':model,
                    **representative,'exit_code':status,'systemd_result':reason,'classification':classification,
                    'verification_memory_max_bytes':16*1024**3,
                    'scope':'Verification holds candidate and independent selection reference; its OOM does not alone establish the HTTP server memory requirement.'})
        if classification=='oracle-mismatch':
            self.event({'record_type':'correctness-quarantine','dataset':dataset['id'],'model':model,'pod':pod,'reason':'independent oracle mismatch'})
            raise RuntimeError('verification oracle mismatch; source quarantined')
        return classification

    def churn(self,dataset,corpus,model):
        label=f'{dataset["id"]}-{model}-dedicated-churn'
        group={'cache_pods':1,'cache_source_bytes':2*1024**3,'max_pod_bytes':2*1024**3}
        if (corpus/'updates').exists():shutil.rmtree(corpus/'updates')
        raw=self.results/f'{label}-requests.jsonl'
        stages=[('grant',0),('revoke',-1),('probe-revoked',None),('probe-revoked',None),('grant',1),('revoke',-1),('grant',1)]
        failed=False; definite_churn_mismatch=False
        try:
            self.start_server(corpus,group,16,1,label)
            for index,(state,delta) in enumerate(stages):
                if index==3:
                    self.stop_server();self.start_server(corpus,group,16,1,label+'-restart')
                command=[str(self.binary),'churn','--corpus',str(corpus),'--auth-dir',str(self.auth),'--state',state,
                         '--mutation-id',f'churn-{index}','--timeout-ms','30000']
                if delta is not None:command+=['--expected-delta',str(delta)]
                if index==2 and dataset['pods']>1:command+=['--evict-pod','1']
                stage=self.results/f'{label}-stage{index}.jsonl'
                code=self.run(command,stage,timeout=120)
                with raw.open('ab') as output,stage.open('rb') as source:shutil.copyfileobj(source,output)
                stage_tail=tail(stage)
                stage.unlink()
                if code:
                    failed=True
                    stage_rows=[]
                    for line in stage_tail.splitlines():
                        try:stage_rows.append(json.loads(line))
                        except json.JSONDecodeError:pass
                    definite_churn_mismatch=any(
                        (row.get('record_type')=='negative-policy-probe' and row.get('status')==200) or
                        (row.get('record_type')=='authorization-probe' and row.get('outcome')=='ok' and row.get('count')!=str(row.get('expected_calendar_records'))) or
                        (row.get('record_type')=='request' and row.get('outcome')=='ok' and (not row.get('mutation_receipt') or 'expected effective change' in stage_tail))
                        for row in stage_rows)
                    break
        finally:self.stop_server()
        audit=self.results/f'{label}-audit.jsonl'
        code=self.run([str(self.binary),'audit','--corpus',str(corpus),'--cache-pods','1','--cache-bytes',str(2*1024**3),'--max-pod-bytes',str(2*1024**3)],audit)
        joined=reconcile(raw,audit) if not failed and code==0 else {'passed':False,'error':'churn probe or audit failed'}
        write_json(self.results/f'{label}-reconciliation.json',joined)
        self.event({'record_type':'dedicated-policy-churn','dataset':dataset['id'],'model':model,'passed':joined['passed'],
                    'scope':'generated inherited calendar plus private exceptions; wrong owner/recipient/anonymous denied; revoke after eviction/restart; actual expected grant/revoke deltas'})
        audit_mismatch='replayed record mismatch' in tail(audit)
        compress(raw);compress(audit)
        if not joined['passed']:
            self.archive_journals(corpus,label)
            if definite_churn_mismatch or audit_mismatch or (not failed and code==0):
                self.event({'record_type':'correctness-quarantine','dataset':dataset['id'],'model':model,'reason':'dedicated policy oracle or durable receipt mismatch'})
                raise RuntimeError('dedicated policy correctness failure; source quarantined')
            self.event({'record_type':'dataset-not-capacity-eligible','dataset':dataset['id'],'model':model,'reason':'policy lane resource/transport/admission failure; incorrect result not inferred'})
            return False
        shutil.rmtree(corpus/'updates')
        return True

    def execute(self):
        outer=int(self.spec['host']['physical_memory_gib']*1024**3*self.spec['host']['overall_memory_fraction'])
        subprocess.run(['sudo','systemctl','set-property','--runtime','sparq-pod-bench.slice',f'MemoryMax={outer}','MemorySwapMax=0'],check=True)
        indexed=self.binary.with_name('indexed_population_preview')
        for model in ['wac','acp']:
            directory=self.corpora/f'indexed-history8-{model}'
            log=self.results/f'indexed-history8-{model}.jsonl'
            code=self.run(['/usr/bin/time','-v','-o',str(self.results/f'indexed-history8-{model}-time.txt'),str(indexed),
                          '--output-dir',str(directory),'--profile','history','--model',model,'--pods','8'],log)
            indexed_tail=tail(log)
            indexed_wrong=any(marker in indexed_tail for marker in ['differs from memory reference','persisted graph structure changed','count oracle mismatch','disagrees with neutral record-count oracle'])
            self.event({'record_type':'indexed-component-diagnostic','model':model,'pods':8,'exit_code':code,'classification':'wrong-result' if indexed_wrong else 'resource-or-diagnostic-error' if code else 'complete','scope':'component only; no HTTP or population capacity claim'})
            manifest=self.results/f'indexed-history8-{model}-files.jsonl'
            with manifest.open('w') as output:
                for base,_,names in os.walk(directory):
                    for name in sorted(names):
                        path=Path(base)/name
                        output.write(json.dumps({'path':str(path.relative_to(directory)),'bytes':path.stat().st_size,'sha256':sha(path)})+'\n')
            compress(manifest);compress(log)
            if indexed_wrong:
                self.event({'record_type':'correctness-quarantine','reason':'indexed representation/reference mismatch','model':model})
                raise RuntimeError('indexed component wrong result; source quarantined and indexes retained')
            if directory.exists():shutil.rmtree(directory)
        for dataset in self.spec['corpora']:
            self.guard(); corpora={}; created=[]
            for model in dataset['models']:
                corpus=self.corpora/f'{dataset["id"]}-{model}'; corpora[model]=corpus; created.append(corpus)
                label=f'{dataset["id"]}-{model}'
                status=self.run([str(self.binary),'pack','--corpus',str(corpus),'--profile',dataset['profile'],
                                 '--pods',str(dataset['pods']),'--model',model]+(['--config-file',str(self.root/dataset['config_file'])] if dataset.get('config_file') else []),self.results/f'{label}-pack.jsonl')
                if status: raise RuntimeError('corpus packing failed; incomplete population not counted')
                shutil.copyfile(corpus/'manifest.json',self.results/f'{label}-manifest.json')
                # Inventory stays uncompressed with the live corpus; collected copy is compressed.
                subprocess.run(['zstd','-q','-f',str(corpus/'pod-summaries.jsonl'),'-o',str(self.results/f'{label}-pod-summaries.jsonl.zst')],check=True)
                subprocess.run(['du','-B1',str(corpus)],stdout=(self.results/f'{label}-disk.txt').open('w'),check=True)
                selected=representatives(corpus/'pod-summaries.jsonl')
                write_json(self.results/f'{label}-verification-sample.json',{'rule':'minimum/maximum source bytes in every observed intensity class; deterministic Pod-ID tiebreak','representatives':selected})
                classifications=[self.verify_representative(dataset,corpus,model,row) for row in selected]
                if any(value!='verified' for value in classifications):
                    self.event({'record_type':'dataset-not-capacity-eligible','dataset':dataset['id'],'model':model,
                                'classifications':classifications,'restriction':'At least one observed volume-class representative is unverified; normal-class checks remain diagnostics only. All Pods remain in persisted inventory.'})
                    corpora[model]=None
            for model,corpus in corpora.items():
                if corpus and not self.churn(dataset,corpus,model):corpora[model]=None
            for group in self.spec['groups']:
                if dataset['id'] not in group['datasets']: continue
                for memory,cpus in itertools.product(group['memory_gib'],group['cpus']):
                    tier=[]
                    for rate in group.get('rates',['derived']):
                        outcomes=[]
                        for replicate in range(group['repeat']):
                            for model in (['wac','acp'] if replicate%2==0 else ['acp','wac']):
                                if not corpora.get(model): continue
                                outcome=self.cell(dataset,corpora[model],model,group,memory,cpus,rate,replicate)
                                outcomes.append(outcome)
                                if outcome.get("dataset_quarantined"):corpora[model]=None
                        tier.extend(outcomes)
                        if outcomes and len(outcomes)==2*group['repeat'] and all(not r['passes_local_guard'] and not r['client_limited'] for r in outcomes):
                            self.event({'record_type':'larger-rate-grid-stop','group':group['id'],'dataset':dataset['id'],'rate':rate,'memory':memory,'cpus':cpus})
                            break
                    if group['id']=='retained-history-hot' and tier and len(tier)==2*group['repeat'] and all(r['passes_local_guard'] for r in tier):
                        self.event({'record_type':'larger-memory-grid-stop','group':group['id'],'memory':memory})
                        break
            # Artifacts retain complete inventories and pack/index hashes; reclaim data only after all dataset cells.
            for corpus in created:
                if corpus and corpus.exists(): shutil.rmtree(corpus)


def validate(path):
    """Static, read-only preflight: no binaries, directories, cloud or Linux calls."""
    spec=json.loads(path.read_text())
    root=path.resolve().parents[3]
    workload=json.loads((root/'bench/ac/million/workload.json').read_text())
    datasets={d['id']:d for d in spec['corpora']}
    assert len(datasets)==len(spec['corpora']), 'duplicate dataset ID'
    assert set(spec['host']['server_cpu_pool']).isdisjoint(spec['host']['client_cpu_pool']), 'CPU pools overlap'
    assert max(spec['host']['server_cpu_pool']+spec['host']['client_cpu_pool'])<spec['host']['physical_vcpus'], 'CPU outside host'
    assert not spec['host']['swap'], 'swap disabled'
    per_active=sum(j['actions_per_active_person_day']*j['logical_reads_per_action']*(1-j['client_cache_hit_fraction'])*j['pod_fanout']/j['batch_factor'] for j in workload['journeys'])
    cells=0; seconds=0.0; estimate=[]
    for dataset in datasets.values():
        assert dataset['pods']>0 and set(dataset['models'])=={'wac','acp'}, 'positive paired population'
        if dataset.get('config_file'):
            config=json.loads((root/dataset['config_file']).read_text())
            assert config['history_months']>0 and config['schema_version']==1, 'corpus config'
    for group in spec['groups']:
        assert group['lane']=='journeys', 'implemented lane required'
        assert group['repeat']<=len(spec['seeds']), 'insufficient seeds'
        assert group['selection'] in ['uniform','skew80-20'], 'unknown target distribution'
        assert max(group['memory_gib'])<spec['host']['physical_memory_gib']*spec['host']['overall_memory_fraction'], 'server tier leaves no client/build budget'
        assert max(group['cpus'])<=len(spec['host']['server_cpu_pool']), 'server CPU tier'
        for dataset_id in group['datasets']:
            dataset=datasets[dataset_id]
            lower_rate=(math.ceil(dataset['pods']*workload['population']['daily_active_fraction'])*per_active+dataset['pods']*workload['execution_v2']['background_read_requests_per_hosted_day'])/86400*workload['offered_multiplier_scenarios'][group['scenario']]
            for memory,cpus,rate in itertools.product(group['memory_gib'],group['cpus'],group.get('rates',['derived'])):
                rate=lower_rate if rate=='derived' else rate
                assert rate>0, 'positive rate'
                count=2*group['repeat']
                duration=spec['measurement']['warmup_seconds']+max(spec['measurement']['measurement_seconds'],spec['measurement']['minimum_offered']/rate)+spec['measurement']['request_timeout_ms']/1000
                cells+=count;seconds+=count*duration
                estimate.append({'group':group['id'],'dataset':dataset_id,'memory_gib':memory,'cpus':cpus,'runs':count,'measurement_and_warmup_upper_seconds':count*duration})
    return {'validated':True,'planned_runs_without_adaptive_stops':cells,'measurement_and_warmup_upper_seconds':seconds,
            'estimate_basis':'Derived-rate estimate ignores positive mutation demand, conservatively lengthening minimum-count runs; fixed-rate cells exact duration floor. Poisson variation, build, generation, correctness, audit, startup and teardown are additional and must fit runtime guard.',
            'runtime_ceiling_seconds':spec['stop_rules']['runtime_ceiling_seconds'],'groups':estimate}


def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--campaign',type=Path,required=True)
    parser.add_argument('--validate-only',action='store_true')
    parser.add_argument('--results',type=Path,default=Path('/var/tmp/sparq-pod-study'))
    parser.add_argument('--corpora',type=Path,default=Path('/var/tmp/sparq-pod-corpus'))
    parser.add_argument('--auth',type=Path,default=Path('/var/tmp/sparq-pod-auth'))
    parser.add_argument('--binary',type=Path,default=Path('/var/tmp/sparq-pod-target/release/examples/pod_population_http'))
    args=parser.parse_args()
    preflight=validate(args.campaign)
    if args.validate_only:
        print(json.dumps(preflight,indent=2));return
    campaign=Campaign(args)
    write_json(campaign.results/'campaign-preflight.json',preflight)
    try: campaign.execute()
    except (RuntimeError,TimeoutError) as error:
        campaign.event({'record_type':'campaign-stopped','reason':str(error),'elapsed_seconds':time.monotonic()-campaign.started})
        raise
    finally: campaign.stop_server()


if __name__=='__main__': main()
