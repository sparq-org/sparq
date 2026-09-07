#!/usr/bin/env python3
"""[GPT-6] Receive only a validated bounded evidence list, retaining disk headroom."""
import argparse
import hashlib
import os
import json
from pathlib import Path
import re
import shlex
import signal
import shutil
import subprocess
import tarfile
import tempfile

MAX_TOTAL = 2 * 1024**3
MAX_FILE = 512 * 1024**2
HEADROOM = 2 * 1024**3
REMOTE_ROOT = '/var/tmp/sparq-pod-study'
OWNED = '.native-retrieval-owned.json'

INVENTORY = r'''
import hashlib,json,os,pathlib,sys
root=pathlib.Path('/var/tmp/sparq-pod-study').resolve()
closed=(root/'MANIFEST.sha256').is_file() and any((root/n).exists() for n in ('DONE','FAILED'))
checksums={}
if closed:
    checksums=dict((name,checksum) for checksum,name in (line.split('  ',1) for line in (root/'MANIFEST.sha256').read_text().splitlines()))
    names=list(checksums)
    names+=['MANIFEST.sha256']+[n for n in ('DONE','FAILED') if (root/n).exists()]
else:
    names=[]
    for directory,dirs,files in os.walk(root,followlinks=False):
        dirs[:]=[d for d in dirs if not (pathlib.Path(directory)/d).is_symlink() and not d.startswith('failure-evidence-')]
        for name in files:
            if name.endswith(('.native','.spqa','.bundle','.tmp','.jsonl')): continue
            names.append(str((pathlib.Path(directory)/name).relative_to(root)))
            if len(names)>10000: raise ValueError('inventory count exceeds bound')
if len(names)>10000: raise ValueError('inventory count exceeds bound')
rows=[]
for name in sorted(set(names)):
    path=root/name
    if path.is_symlink() or not path.resolve().is_relative_to(root) or not path.is_file(): raise ValueError('unsafe evidence path')
    st=path.stat()
    checksum=checksums.get(name)
    if closed and checksum is None:
        with path.open('rb') as stream: checksum=hashlib.file_digest(stream,'sha256').hexdigest()
    rows.append(dict(path=name,size=st.st_size,mtime_ns=st.st_mtime_ns,finalized=closed,sha256=checksum))
print(json.dumps(rows))
'''
SENDER = r'''
import json,os,pathlib,sys,tarfile
root=pathlib.Path('/var/tmp/sparq-pod-study').resolve()
rows=json.load(sys.stdin)
with tarfile.open(fileobj=sys.stdout.buffer,mode='w|') as archive:
    for row in rows:
        path=root/row['path']
        if path.is_symlink() or not path.resolve().is_relative_to(root): raise ValueError('unsafe transfer source')
        before=path.stat()
        if (before.st_size,before.st_mtime_ns)!=(row['size'],row['mtime_ns']): raise ValueError('evidence changed before transfer')
        with path.open('rb') as stream:
            info=tarfile.TarInfo(row['path']);info.size=row['size'];info.mtime=0
            archive.addfile(info,stream)
        after=path.stat()
        if (before.st_size,before.st_mtime_ns,before.st_ino)!=(after.st_size,after.st_mtime_ns,after.st_ino): raise ValueError('evidence changed during transfer')
'''


def validate_name(name):
    if not isinstance(name, str) or not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9._/-]{0,400}', name):
        raise ValueError('invalid evidence path')
    if any(part in ('', '.', '..') for part in name.split('/')) or Path(name).suffix in {'.native', '.spqa', '.bundle'}:
        raise ValueError('unsafe or excluded evidence path')


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream,'sha256').hexdigest()


def owned_files(results):
    path=results/OWNED
    return json.loads(path.read_text()) if path.exists() else {}


def save_owned(results, owned):
    temporary=results/(OWNED+'.tmp')
    temporary.write_text(json.dumps(owned,sort_keys=True)+'\n')
    temporary.replace(results/OWNED)


def validate_inventory(rows):
    if not isinstance(rows,list) or len(rows)>10000:
        raise ValueError('invalid inventory count')
    names=set()
    for row in rows:
        name=row['path'];validate_name(name)
        if name in names: raise ValueError('duplicate evidence path')
        names.add(name)
        if type(row['size']) is not int or not 0<=row['size']<=MAX_FILE or type(row['mtime_ns']) is not int or row['mtime_ns']<0:
            raise ValueError('invalid evidence size or timestamp')
        if type(row.get('finalized',False)) is not bool:
            raise ValueError('invalid finalization flag')
        if row.get('finalized') and not re.fullmatch('[0-9a-f]{64}',row.get('sha256') or ''):
            raise ValueError('finalized file lacks its manifest checksum')
    if sum(row['size'] for row in rows)>MAX_TOTAL:
        raise ValueError('receiver aggregate 2 GiB bound exceeded')


def reconcile_final(rows, results):
    """Retire only unchanged, receiver-owned provisional files superseded by a terminal manifest."""
    validate_inventory(rows)
    if not rows or not all(row.get('finalized') is True for row in rows): return
    required={row['path'] for row in rows}
    if 'MANIFEST.sha256' not in required or not required.intersection({'DONE','FAILED'}):
        raise ValueError('final inventory needs a terminal marker and manifest')
    local_bytes(results)  # Reject local symlinks before considering ownership.
    owned=owned_files(results); reusable={}
    for name, checksum in owned.items():
        validate_name(name)
        path=results/name
        if name not in required and path.is_file() and digest(path)==checksum:
            reusable.setdefault(checksum,[]).append(name)
    retired=[]
    wanted_hashes={row["path"]:row["sha256"] for row in rows}
    for row in rows:
        destination=results/row['path']
        matches=reusable.get(row['sha256'],[])
        if not destination.exists() and matches:
            original=matches.pop();destination.parent.mkdir(parents=True,exist_ok=True)
            (results/original).replace(destination)
            os.utime(destination,ns=(row['mtime_ns'],row['mtime_ns']))
            owned.pop(original);owned[row['path']]=row['sha256']
            retired.append(dict(path=original,sha256=row['sha256'],retained_as=row['path']))
    for name, checksum in list(owned.items()):
        path=results/name
        if (name not in required or wanted_hashes.get(name)!=checksum) and path.is_file() and digest(path)==checksum:
            path.unlink();owned.pop(name)
            retired.append(dict(path=name,sha256=checksum,reason='superseded provisional snapshot differs from terminal manifest'))
    save_owned(results,owned)
    if retired:
        with (results/'.native-retrieval-retired.jsonl').open('a') as stream:
            for record in retired: stream.write(json.dumps(record,sort_keys=True)+'\n')


def local_bytes(results):
    total = 0
    for path in results.rglob('*'):
        if path.is_symlink():
            raise ValueError('local result symlinks are forbidden')
        if path.is_file(): total += path.stat().st_size
    return total


def received_bytes(results):
    # The transfer ceiling covers our evidence and replacement scratch. Locally
    # authored source/config/cost artifacts are protected; free-space checks still
    # account for every byte on their filesystem.
    local_bytes(results)
    return sum((results/name).stat().st_size for name in owned_files(results) if (results/name).is_file())


def plan(rows, results, free_bytes):
    validate_inventory(rows)
    selected=[]
    current=received_bytes(results); peak=current; projected=current
    for row in rows:
        target=results/row['path']
        if target.exists() and not target.is_file(): raise ValueError('local path is not a regular file')
        old=target.stat() if target.exists() else None
        if old and (old.st_size,old.st_mtime_ns)==(row['size'],row['mtime_ns']):
            if not row.get('sha256') or digest(target)==row['sha256']: continue
        selected.append(row)
        peak=max(peak,projected+row['size'])
        projected+=row['size']-(old.st_size if old else 0)
    if peak>MAX_TOTAL:
        raise ValueError('receiver aggregate 2 GiB bound exceeded, including replacement scratch')
    if free_bytes<HEADROOM+peak-current:
        raise ValueError('receiver must retain 2 GiB free headroom')
    return selected


def ssh_command(host, script):
    return ['ssh', '-i', host['key_path'], '-o', 'StrictHostKeyChecking=yes', '-o',
        'UserKnownHostsFile=' + host['known_hosts'], '-o', 'ConnectTimeout=15',
        '-o', 'ServerAliveInterval=15', '-o', 'ServerAliveCountMax=2',
        'ubuntu@' + host['ip'], 'python3 -c ' + shlex.quote(script)]


def receive(process, selected, results):
    current = received_bytes(results)
    owned=owned_files(results)
    with tempfile.TemporaryDirectory(prefix='native-transfer-', dir=results.parent) as directory:
        with tarfile.open(fileobj=process.stdout, mode='r|') as archive:
            for row in selected:
                member = archive.next()
                if member is None or not member.isfile() or member.name != row['path'] or member.size != row['size']:
                    raise ValueError('transfer differs from validated file list')
                if current + member.size > MAX_TOTAL or shutil.disk_usage(results).free < HEADROOM + member.size:
                    raise ValueError('receiver aggregate/headroom changed during transfer')
                temporary = Path(directory) / 'file'
                with archive.extractfile(member) as source, temporary.open('wb') as destination:
                    remaining = member.size
                    while remaining:
                        block = source.read(min(1024**2, remaining))
                        if not block: raise ValueError('truncated evidence file')
                        destination.write(block); remaining -= len(block)
                target = results / row['path']; target.parent.mkdir(parents=True, exist_ok=True)
                checksum=digest(temporary)
                if row.get('sha256') and checksum!=row['sha256']:
                    raise ValueError('received bytes differ from the terminal manifest')
                old_size = target.stat().st_size if target.exists() else 0
                if target.exists() and (row['path'] not in owned or digest(target)!=owned[row['path']]):
                    raise ValueError('refusing to overwrite a locally created or modified artifact')
                temporary.replace(target)
                os.utime(target, ns=(row['mtime_ns'], row['mtime_ns']))
                owned[row['path']]=checksum;save_owned(results,owned)
                current=received_bytes(results)
            if archive.next() is not None:
                raise ValueError('sender emitted an unselected artifact')


def pull(host, results):
    inventory = subprocess.run(ssh_command(host, INVENTORY), capture_output=True, text=True, check=True, timeout=60)
    rows = json.loads(inventory.stdout)
    reconcile_final(rows,results)
    selected = plan(rows, results, shutil.disk_usage(results).free)
    if not selected: return
    process = subprocess.Popen(ssh_command(host, SENDER), stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
    try:
        process.stdin.write(json.dumps(selected).encode()); process.stdin.close()
        receive(process, selected, results)
        if process.wait(timeout=30): raise ValueError('sender did not complete a stable evidence snapshot')
    finally:
        if process.poll() is None: process.kill()
        process.wait()


if __name__ == '__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--host',type=Path,required=True)
    parser.add_argument('--results',type=Path,required=True)
    args=parser.parse_args()
    def timed_out(_signal, _frame):
        raise TimeoutError('bounded native evidence transfer exceeded 300 seconds')
    signal.signal(signal.SIGALRM,timed_out)
    signal.alarm(300)
    try:
        pull(json.loads(args.host.read_text()),args.results)
    finally:
        signal.alarm(0)
