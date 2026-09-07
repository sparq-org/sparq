#!/usr/bin/env python3
"""[GPT-6] Ready-only native host lifecycle; never dispatches an experiment."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import tempfile
import subprocess
import time

RESULTS = Path('/var/tmp/sparq-pod-study')
CONTROL = Path('/var/tmp/sparq-native-control')
DATA = Path('/mnt/sparq-native/data0')
MAX_EVIDENCE_BYTES = 2 * 1024**3


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def receipt_identity(receipt):
    if receipt.get('status') == 'not-run' and isinstance(receipt.get('reason'), str) and receipt['reason'].strip():
        return
    if receipt.get('status') not in ('completed', 'stopped-with-partial-evidence'):
        raise ValueError('completion receipt needs an explicit terminal status')
    for key, length in (('source_commit', 40), ('campaign_sha256', 64), ('binary_sha256', 64)):
        if not re.fullmatch('[0-9a-f]{' + str(length) + '}', receipt.get(key, '')):
            raise ValueError('completion receipt lacks exact source/campaign/binary identity')
    for key in ('source_path', 'campaign_path', 'binary_path'):
        path = Path(receipt[key]).resolve()
        if not path.is_relative_to(DATA.resolve()) or not path.exists():
            raise ValueError('measured source inputs must be retained on the declared data0 filesystem')
    revision = subprocess.check_output(['git', '-C', receipt['source_path'], 'rev-parse', 'HEAD'], text=True).strip()
    if revision != receipt['source_commit']:
        raise ValueError('completion source differs from its checkout')
    if subprocess.check_output(['git', '-C', receipt['source_path'], 'status', '--porcelain']):
        raise ValueError('completion source checkout is dirty')
    for stem in ('campaign', 'binary'):
        if digest(Path(receipt[stem + '_path'])) != receipt[stem + '_sha256']:
            raise ValueError('completion input checksum mismatch')
    campaign = json.loads(Path(receipt['campaign_path']).read_text())
    if campaign.get('status') != 'frozen-before-measurement':
        raise ValueError('completion campaign was not frozen')


def child_services_stopped():
    output = subprocess.check_output(['systemctl', 'list-units', '--all', '--plain', '--no-legend',
        '--state=active,activating,reloading,deactivating', 'sparq-pod-preloaded-*.service',
        'sparq-native-job-*.service'], text=True)
    return not output.strip()


def evidence_files():
    files = []
    for path in sorted(RESULTS.rglob('*')):
        if path.is_symlink():
            raise ValueError('result symlinks are forbidden')
        if path.is_file() and str(path.relative_to(RESULTS)) not in {'DONE', 'FAILED', 'READY', 'MANIFEST.sha256', 'MANIFEST.tmp'}:
            if path.suffix in {'.native', '.spqa', '.bundle'}:
                raise ValueError('large source or native archives must stay outside retrieved results')
            files.append(path)
    if any(p.stat().st_size > 512 * 1024**2 for p in files):
        raise ValueError('each retrieved evidence file must be at most 512 MiB')
    if sum(p.stat().st_size for p in files) > MAX_EVIDENCE_BYTES:
        raise ValueError('result evidence exceeds the 2 GiB retrieval bound')
    return files


def finalize(marker):
    (RESULTS / 'finished-at.txt').write_text(time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()) + '\n')
    rows = [f'{digest(p)}  {p.relative_to(RESULTS)}\n' for p in evidence_files()]
    (RESULTS / 'MANIFEST.tmp').write_text(''.join(rows))
    (RESULTS / 'MANIFEST.tmp').replace(RESULTS / 'MANIFEST.sha256')
    (RESULTS / 'READY').unlink(missing_ok=True)
    (RESULTS / marker).touch()


def finalize_failure(error):
    """Publish bounded immutable diagnostic snapshots without re-running a failed gate."""
    RESULTS.mkdir(parents=True, exist_ok=True)
    (RESULTS / 'failure-detail.txt').write_text(str(error)[:4096] + '\n')
    (RESULTS / 'finished-at.txt').write_text(time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()) + '\n')
    candidates = []
    exclusions = []
    for directory, dirs, files in os.walk(RESULTS, followlinks=False):
        for d in dirs:
            if (Path(directory) / d).is_symlink():
                exclusions.append(dict(path=str((Path(directory) / d).relative_to(RESULTS)), reason='symlink directory excluded'))
        dirs[:] = [d for d in dirs if not d.startswith('failure-evidence-') and not (Path(directory) / d).is_symlink()]
        for name in files:
            path = Path(directory) / name
            if str(path.relative_to(RESULTS)) in {'READY', 'DONE', 'FAILED', 'MANIFEST.sha256', 'MANIFEST.tmp', 'failure-detail.txt', 'finished-at.txt'}:
                continue
            candidates.append(path)
            if len(candidates) >= 10000:
                exclusions.append(dict(path='remaining entries', reason='10000-entry diagnostic bound'))
                break
        if len(candidates) >= 10000:
            break
    snapshots = Path(tempfile.mkdtemp(prefix='failure-evidence-', dir=RESULTS))
    admitted = []
    used = 0
    # Keep small structured evidence before potentially large request streams.
    candidates.sort(key=lambda p: (p.suffix != '.json', str(p)))
    for source in candidates:
        relative = str(source.relative_to(RESULTS))
        destination = snapshots / relative
        try:
            if source.is_symlink() or not source.is_file() or not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9._/-]{0,400}', relative):
                raise ValueError('unsafe or unsupported diagnostic path')
            before = source.stat()
            if source.suffix in {'.native', '.spqa', '.bundle'} or before.st_size > 512 * 1024**2:
                raise ValueError('payload or oversized diagnostic excluded')
            if used + before.st_size > MAX_EVIDENCE_BYTES - 16 * 1024**2:
                raise ValueError('aggregate diagnostic bound')
            destination.parent.mkdir(parents=True, exist_ok=True)
            with source.open('rb') as inp, destination.open('xb') as out:
                remaining = before.st_size
                while remaining:
                    block = inp.read(min(1024**2, remaining))
                    if not block:
                        raise ValueError('source became shorter while snapshotting')
                    out.write(block); remaining -= len(block)
            after = source.stat()
            if (before.st_size, before.st_mtime_ns, before.st_ino) != (after.st_size, after.st_mtime_ns, after.st_ino):
                raise ValueError('source changed while snapshotting')
            admitted.append(destination)
            used += before.st_size
        except (OSError, ValueError) as failure:
            destination.unlink(missing_ok=True)
            exclusions.append(dict(path=relative[:401], reason=str(failure)[:512]))
    report = RESULTS / 'failure-evidence.json'
    report.write_text(json.dumps(dict(scope='immutable diagnostic snapshots; benchmark admission remains independent',
        bytes=used, snapshots=len(admitted), exclusions=exclusions), indent=2) + '\n')
    admitted += [report, RESULTS / 'failure-detail.txt', RESULTS / 'finished-at.txt']
    rows = [f'{digest(p)}  {p.relative_to(RESULTS)}\n' for p in admitted]
    (RESULTS / 'MANIFEST.tmp').write_text(''.join(rows))
    (RESULTS / 'MANIFEST.tmp').replace(RESULTS / 'MANIFEST.sha256')
    (RESULTS / 'READY').unlink(missing_ok=True)
    (RESULTS / 'DONE').unlink(missing_ok=True)
    (RESULTS / 'FAILED').touch()


def serve(deadline):
    RESULTS.mkdir(parents=True, exist_ok=True)
    CONTROL.mkdir(exist_ok=True)
    if not (RESULTS / 'native-storage.json').exists() or not DATA.is_mount():
        raise ValueError('verified native storage setup is required')
    identity = dict(scope='ready-only; no automatic campaign dispatch', deadline_epoch=deadline,
        launcher_source_commit=subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(),
        source_bundle_sha256=digest(Path('/var/tmp/sparq.bundle')),
        corpus_filesystem=str(DATA), unallocated_filesystem='/mnt/sparq-native/data1',
        required_job_unit_prefix='sparq-native-job-', result_limit_bytes=MAX_EVIDENCE_BYTES)
    (RESULTS / 'native-host-scope.json').write_text(json.dumps(identity, indent=2) + '\n')
    with (RESULTS / 'environment.txt').open('w') as stream:
        for argv in (['uname', '-a'], ['lscpu'], ['lsblk', '--json', '--bytes', '--paths'],
                     ['findmnt', '--json'], ['df', '-B1'], ['cat', '/proc/meminfo'], ['swapon', '--show']):
            subprocess.run(argv, stdout=stream, stderr=subprocess.STDOUT, check=True, timeout=60)
    (RESULTS / 'stage.txt').write_text('ready; waiting for explicitly reviewed source and frozen campaign\n')
    (RESULTS / 'READY').touch()
    while time.time() < deadline - 600:
        request = CONTROL / 'FINISH.json'
        if request.exists():
            receipt = json.loads(request.read_text())
            receipt_identity(receipt)
            if not child_services_stopped():
                raise ValueError('child services still running at finalization request')
            (RESULTS / 'completion-receipt.json').write_text(json.dumps(receipt, indent=2) + '\n')
            evidence_files()
            finalize('DONE')
            return
        evidence_files()
        print('native host ready; campaign dispatch remains explicit', flush=True)
        time.sleep(45)
    raise ValueError('host deadline reached without an explicit completion receipt')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--deadline', type=int, required=True)
    args = parser.parse_args()
    try:
        serve(args.deadline)
    except Exception as error:
        finalize_failure(error)
        raise
