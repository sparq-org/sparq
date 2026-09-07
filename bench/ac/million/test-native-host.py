#!/usr/bin/env python3
"""[GPT-6] Hermetic native-host safety checks. No AWS or real block devices."""
import io
import tarfile
import sys
from types import SimpleNamespace
from decimal import Decimal
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

HERE = Path(__file__).parent

def load(name):
    spec = importlib.util.spec_from_file_location(name.replace('-', '_'), HERE / (name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module

storage, host, cost, retrieval = (load(name) for name in ('native-storage', 'native-host', 'native-cost', 'native-retrieve'))


def disk(number, **changes):
    return dict(path=f'/dev/nvme{number}n1', type='disk', model=storage.MODEL,
                serial=f'AWS{number}', size=1_425_000_000_000, fstype=None, mountpoints=[None], **changes)


def topology():
    root = dict(path='/dev/nvme0n1', type='disk', model='Amazon Elastic Block Store', serial='vol000',
                size=80 * 1024**3, fstype=None, mountpoints=[None], children=[dict(mountpoints=['/'])])
    return dict(blockdevices=[root, disk(1), disk(2)])


class NativeHostTests(unittest.TestCase):
    def signatures(self):
        return {f'/dev/nvme{i}n1': dict(signatures=[]) for i in (1, 2)}

    def test_plan_excludes_ebs_root_and_requires_both_blank_instance_disks(self):
        self.assertEqual([d['path'] for d in storage.plan(topology(), self.signatures())], ['/dev/nvme1n1', '/dev/nvme2n1'])
        for change in (dict(fstype='ext4'), dict(mountpoints=['/']), dict(children=[{}]),
                       dict(path='/dev/sda'), dict(serial='vol123'), dict(size=80 * 1024**3)):
            candidate = topology(); candidate['blockdevices'][1].update(change)
            with self.subTest(change=change), self.assertRaises(ValueError):
                storage.plan(candidate, self.signatures())
        signatures = self.signatures(); signatures['/dev/nvme2n1']['signatures'] = [dict(type='ext4')]
        with self.assertRaises(ValueError):
            storage.plan(topology(), signatures)
        with self.assertRaises(ValueError):
            storage.plan(dict(blockdevices=topology()['blockdevices'][:2]), self.signatures())

    def test_no_filesystem_mutation_before_all_devices_admitted(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with patch.object(storage, 'MOUNT', root / 'mount'), patch.object(storage.os, 'geteuid', return_value=0), \
                 patch.object(storage, 'snapshot', return_value=topology()), \
                 patch.object(storage, 'command', side_effect=[json.dumps(dict(signatures=[])), json.dumps(dict(signatures=[dict(type='ext4')]))]), \
                 patch.object(storage.subprocess, 'run') as run:
                with self.assertRaises(ValueError): storage.prepare(root / 'receipt.json')
                run.assert_not_called()

    def test_ambiguous_wipefs_output_never_formats_a_device(self):
        for invalid in ({}, {'signatures':None}, {'signatures':False}, {'signatures':0}, {'signatures':{}}, {'signatures':''}):
            with self.subTest(output=invalid), tempfile.TemporaryDirectory() as directory:
                root=Path(directory)
                with patch.object(storage,'MOUNT',root/'mount'), patch.object(storage.os,'geteuid',return_value=0), \
                     patch.object(storage,'snapshot',return_value=topology()), \
                     patch.object(storage,'command',side_effect=[json.dumps({'signatures':[]}),json.dumps(invalid)]), \
                     patch.object(storage.subprocess,'run') as run:
                    with self.assertRaises(ValueError): storage.prepare(root/'receipt.json')
                    run.assert_not_called()

    def test_success_formats_only_validated_devices_and_second_invocation_rejects(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with patch.object(storage, 'MOUNT', root / 'mount'), patch.object(storage.os, 'geteuid', return_value=0), \
                 patch.object(storage, 'snapshot', return_value=topology()), \
                 patch.object(storage, 'command', return_value=json.dumps(dict(signatures=[]))), \
                 patch.object(storage.subprocess, 'run') as run:
                storage.prepare(root / 'receipt.json')
                formats = [c.args[0] for c in run.call_args_list if c.args[0][0] == 'mkfs.ext4']
                self.assertEqual([c[-1] for c in formats], ['/dev/nvme1n1', '/dev/nvme2n1'])
                with self.assertRaises(ValueError): storage.prepare(root / 'receipt.json')
                self.assertEqual(len([c for c in run.call_args_list if c.args[0][0] == 'mkfs.ext4']), 2)

    def test_changed_topology_stops_before_format(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); changed = topology(); changed['blockdevices'].pop()
            with patch.object(storage, 'MOUNT', root / 'mount'), patch.object(storage.os, 'geteuid', return_value=0), \
                 patch.object(storage, 'snapshot', side_effect=[topology(), changed]), \
                 patch.object(storage, 'command', return_value=json.dumps(dict(signatures=[]))), \
                 patch.object(storage.subprocess, 'run') as run:
                with self.assertRaises(ValueError): storage.prepare(root / 'receipt.json')
                run.assert_not_called()

    def test_native_constraints_precede_cloud_commands(self):
        for changes in ({'SPARQ_POD_INSTANCE_TYPE':'r7g.12xlarge'}, {'SPARQ_POD_VOLUME_GB':'81'},
                        {'SPARQ_POD_WATCHDOG_SECONDS':'43201'}):
            result = subprocess.run(['/bin/bash', str(HERE / 'native-host.sh')],
                env=dict(os.environ, SPARQ_POD_PRIOR_AWS_USD='25', **changes), capture_output=True, text=True, timeout=5)
            self.assertNotEqual(result.returncode, 0)
            self.assertNotIn('orphan check', result.stderr)

    def test_native_supervisor_requires_exact_token_and_confirms_termination(self):
        for mode in ('match','wrong-tag','invalid-token'):
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as directory:
                root=Path(directory); commands=root/'bin';commands.mkdir()
                token='sparq-pod-native-20260907T000000Z-123'
                (root/'instance-id.txt').write_text('i-1234')
                (root/'study-run-token.txt').write_text(token if mode!='invalid-token' else token+'-extra')
                (root/'price-checked-at.txt').write_text('2000-01-01T00:00:00Z')
                (root/'watchdog-seconds.txt').write_text('60')
                (commands/'sleep').write_text('#!/bin/bash\nexit 0\n');(commands/'sleep').chmod(0o755)
                (commands/'aws').write_text(r'''#!/bin/bash
printf '%s\n' "$*" >> "$FIXTURE/trace"
[[ "$*" == *'--instance-ids i-1234'* ]] || exit 9
if [[ "$*" == *describe-instances* ]]; then
  [[ "$*" == *"Name=tag:study-run,Values=$EXPECTED_TOKEN"* ]] || exit 9
  [[ "$*" == *'Name=tag:purpose,Values=sparq-bench'* ]] || exit 9
  if [[ "$TEST_MODE" == wrong-tag ]]; then printf 'None\n'
  elif [[ -e "$FIXTURE/terminated" ]]; then printf 'terminated\n'
  else printf 'running\n'; fi
elif [[ "$*" == *terminate-instances* ]]; then touch "$FIXTURE/terminated"; printf 'shutting-down\n'
else exit 9; fi
''')
                (commands/'aws').chmod(0o755)
                result=subprocess.run(['/bin/bash',str(HERE/'supervise-instance.sh'),str(root)],
                    env=dict(os.environ,PATH=str(commands)+':'+os.environ['PATH'],FIXTURE=str(root),EXPECTED_TOKEN=token,TEST_MODE=mode),
                    capture_output=True,text=True,timeout=5)
                self.assertEqual(result.returncode==0,mode=='match',result.stderr)
                self.assertEqual((root/'terminated').exists(),mode=='match')
                if mode=='match': self.assertIn('supervisor-complete',result.stdout)
                if mode=='invalid-token': self.assertFalse((root/'trace').exists())

    def test_native_price_keeps_one_prior_reserve_and_fails_over_total_budget(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'price.csv'
            path.write_text('"SKU","Region Code","Volume API Name","Product Family","TermType","Unit","Currency","PricePerUnit"\n'
                            '"fixture","eu-west-2","gp3","Storage","OnDemand","GB-Mo","USD","0.0928"\n')
            with patch.object(cost.build.price, 'bulk_csv_hourly_price', return_value=(Decimal('3.8328'), {})):
                result = cost.reservation(path, Decimal(25))
                self.assertEqual(Decimal(result['maximum_accumulated_study_spend']), Decimal(25) + Decimal(result['maximum_planned_run']))
                self.assertLess(Decimal(result['maximum_accumulated_study_spend']), Decimal(100))
                self.assertFalse(result['invoice_verified'])
                with self.assertRaises(cost.build.price.PricingError): cost.reservation(path, Decimal(99))

    def test_completion_requires_identity_and_stopped_children(self):
        for receipt in ({}, dict(status='completed'), dict(status='not-run', reason='')):
            with self.assertRaises(ValueError): host.receipt_identity(receipt)
        host.receipt_identity(dict(status='not-run', reason='explicitly cancelled before execution'))
        with patch.object(host.subprocess, 'check_output', return_value='sparq-native-job-test.service loaded active running\n'):
            self.assertFalse(host.child_services_stopped())
        with patch.object(host.subprocess, 'check_output', return_value=''):
            self.assertTrue(host.child_services_stopped())

    def test_completed_receipt_rechecks_source_hashes_and_freeze(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(host, 'DATA', Path(directory)):
            root = Path(directory); source = root / 'source'; source.mkdir()
            campaign = source / 'campaign.json'; campaign.write_text('{"status":"frozen-before-measurement"}')
            binary = source / 'binary'; binary.write_bytes(b'synthetic binary fixture')
            receipt = dict(status='completed', source_commit='a'*40, source_path=str(source),
                campaign_path=str(campaign), campaign_sha256=host.digest(campaign),
                binary_path=str(binary), binary_sha256=host.digest(binary))
            with patch.object(host.subprocess, 'check_output', side_effect=['a'*40, b'']):
                host.receipt_identity(receipt)
            binary.write_bytes(b'changed binary fixture')
            with patch.object(host.subprocess, 'check_output', side_effect=['a'*40, b'']):
                with self.assertRaisesRegex(ValueError, 'checksum'): host.receipt_identity(receipt)
            receipt['binary_sha256'] = host.digest(binary)
            campaign.write_text('{"status":"unfrozen"}'); receipt['campaign_sha256'] = host.digest(campaign)
            with patch.object(host.subprocess, 'check_output', side_effect=['a'*40, b'']):
                with self.assertRaisesRegex(ValueError, 'frozen'): host.receipt_identity(receipt)

    def test_failure_manifest_survives_invalid_and_oversized_evidence(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(host,'RESULTS',Path(directory)):
            root=Path(directory);(root/'READY').touch();(root/'result.json').write_text('{"closed":true}')
            with (root/'oversized.log').open('wb') as out: out.truncate(513*1024**2)
            (root/'link').symlink_to(root/'result.json')
            with self.assertRaises(ValueError): host.evidence_files()
            host.finalize_failure(ValueError('fixture evidence validation failed'))
            self.assertTrue((root/'FAILED').exists());self.assertFalse((root/'READY').exists())
            entries=(root/'MANIFEST.sha256').read_text().splitlines()
            self.assertTrue(any('/result.json' in row for row in entries))
            self.assertFalse(any('oversized.log' in row for row in entries))
            for row in entries:
                checksum,name=row.split('  ',1);self.assertEqual(checksum,host.digest(root/name))
            excluded=json.loads((root/'failure-evidence.json').read_text())['exclusions']
            self.assertEqual({r['path'] for r in excluded},{'oversized.log','link'})

    def test_receiver_enforces_aggregate_and_free_space_before_transfer(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(retrieval,'MAX_TOTAL',300), \
             patch.object(retrieval,'MAX_FILE',200), patch.object(retrieval,'HEADROOM',100):
            root=Path(directory)
            rows=[dict(path='a.json',size=100,mtime_ns=1)]
            self.assertEqual(retrieval.plan(rows,root,500),rows)
            with self.assertRaisesRegex(ValueError,'headroom'): retrieval.plan(rows,root,150)
            with self.assertRaisesRegex(ValueError,'aggregate'):
                retrieval.plan([dict(path='a.json',size=150,mtime_ns=1),dict(path='b.json',size=151,mtime_ns=1)],root,1000)
            with self.assertRaises(ValueError): retrieval.plan([dict(path='../escape',size=1,mtime_ns=1)],root,1000)
            with self.assertRaises(ValueError): retrieval.plan(rows+rows,root,1000)

    def test_receiver_copies_only_the_declared_files_and_rejects_size_change(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory); rows=[dict(path='cell/result.json',size=2,mtime_ns=1000000000)]
            def stream(size):
                data=io.BytesIO()
                with tarfile.open(fileobj=data,mode='w') as archive:
                    info=tarfile.TarInfo('cell/result.json');info.size=size
                    archive.addfile(info,io.BytesIO(b'{}x'[:size]))
                data.seek(0);return data
            with patch.object(retrieval.shutil,'disk_usage',return_value=SimpleNamespace(free=10*1024**3)):
                retrieval.receive(SimpleNamespace(stdout=stream(2)),rows,root)
                self.assertEqual((root/'cell/result.json').read_bytes(),b'{}')
                with self.assertRaisesRegex(ValueError,'validated file list'):
                    retrieval.receive(SimpleNamespace(stdout=stream(3)),rows,root)
                self.assertEqual((root/'cell/result.json').read_bytes(),b'{}')

    def test_final_reconciliation_keeps_locally_modified_received_files(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);original=root/'old.json';original.write_bytes(b'original received bytes')
            retrieval.save_owned(root,{'old.json':retrieval.digest(original)})
            original.write_bytes(b'local edit must survive')
            rows=[dict(path='FAILED',size=0,mtime_ns=1,finalized=True,sha256='a'*64),
                  dict(path='MANIFEST.sha256',size=1,mtime_ns=1,finalized=True,sha256='b'*64)]
            retrieval.reconcile_final(rows,root)
            self.assertEqual(original.read_bytes(),b'local edit must survive')

    def test_failure_finalization_after_near_cap_live_pull_preserves_local_artifacts(self):
        with tempfile.TemporaryDirectory() as directory:
            base=Path(directory);remote=base/'remote';local=base/'local';remote.mkdir();local.mkdir()
            payload=b'{"data":"'+b'x'*48000+b'"}'
            (remote/'result.json').write_bytes(payload)
            protected={'source-commit.txt':b'local source identity', 'config.json':b'{"local":true}', 'cost-estimate.json':b'{"reserved":true}'}
            for name,content in protected.items(): (local/name).write_bytes(content)
            def inventory():
                script=retrieval.INVENTORY.replace("'/var/tmp/sparq-pod-study'",repr(str(remote)))
                return json.loads(subprocess.check_output([sys.executable,'-c',script],text=True))
            def transfer(rows):
                data=io.BytesIO()
                with tarfile.open(fileobj=data,mode='w') as archive:
                    for row in rows:
                        info=tarfile.TarInfo(row['path']);info.size=row['size']
                        with (remote/row['path']).open('rb') as source: archive.addfile(info,source)
                data.seek(0);retrieval.receive(SimpleNamespace(stdout=data),rows,local)
            with patch.object(retrieval,'MAX_TOTAL',64*1024), patch.object(retrieval,'MAX_FILE',64*1024), \
                 patch.object(retrieval.shutil,'disk_usage',return_value=SimpleNamespace(free=10*1024**3)):
                rows=inventory();transfer(retrieval.plan(rows,local,10*1024**3))
                self.assertGreater(retrieval.received_bytes(local),retrieval.MAX_TOTAL*.7)
                with patch.object(host,'RESULTS',remote): host.finalize_failure(ValueError('fixture failed after live collection'))
                rows=inventory()
                with self.assertRaisesRegex(ValueError,'aggregate'): retrieval.plan(rows,local,10*1024**3)
                retrieval.reconcile_final(rows,local)
                transfer(retrieval.plan(rows,local,10*1024**3))
                self.assertTrue((local/'FAILED').exists())
                self.assertLessEqual(retrieval.received_bytes(local),retrieval.MAX_TOTAL)
                for row in (local/'MANIFEST.sha256').read_text().splitlines():
                    checksum,name=row.split('  ',1);self.assertEqual(checksum,host.digest(local/name))
                for name,content in protected.items(): self.assertEqual((local/name).read_bytes(),content)
                self.assertFalse((local/'result.json').exists())
                self.assertTrue(any(p.read_bytes()==payload for p in local.glob('failure-evidence-*/result.json')))

    def test_nested_manifest_is_complete_and_native_payload_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(host, 'RESULTS', Path(directory)):
            root = Path(directory); (root / 'cell').mkdir(); (root / 'cell' / 'result.json').write_text('{}'); (root / 'cell' / 'MANIFEST.sha256').write_text('fixture child manifest')
            (root / 'READY').touch(); host.finalize('DONE')
            for row in (root / 'MANIFEST.sha256').read_text().splitlines():
                checksum, name = row.split('  ', 1)
                self.assertEqual(checksum, host.digest(root / name))
            self.assertIn('cell/result.json', (root / 'MANIFEST.sha256').read_text())
            self.assertIn('cell/MANIFEST.sha256', (root / 'MANIFEST.sha256').read_text())
            self.assertFalse((root / 'READY').exists())
            (root / 'payload.native').touch()
            with self.assertRaises(ValueError): host.evidence_files()


if __name__ == '__main__': unittest.main()
