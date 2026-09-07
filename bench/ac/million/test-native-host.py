#!/usr/bin/env python3
"""[GPT-6] Hermetic native-host safety checks. No AWS or real block devices."""
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

storage, host, cost = (load(name) for name in ('native-storage', 'native-host', 'native-cost'))


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
