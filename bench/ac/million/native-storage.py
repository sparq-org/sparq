#!/usr/bin/env python3
"""[GPT-6] Provision only two positively identified, blank EC2 instance-store disks."""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess

MOUNT = Path('/mnt/sparq-native')
MODEL = 'Amazon EC2 NVMe Instance Storage'


def command(argv):
    return subprocess.check_output(argv, text=True)


def plan(topology, signatures):
    """Pure fail-closed plan; validate every disk before the first filesystem write."""
    selected = []
    for disk in topology['blockdevices']:
        if str(disk.get('model', '')).strip() != MODEL:
            continue
        name = disk['path']
        if not re.fullmatch(r'/dev/nvme[0-9]+n1', name) or disk.get('type') != 'disk':
            raise ValueError('instance-store identity is not a whole NVMe disk')
        serial = str(disk.get('serial') or '').strip()
        if not serial.startswith('AWS') or serial.lower().startswith('vol'):
            raise ValueError('instance-store serial is missing or unexpected')
        if not 1_400_000_000_000 <= int(disk['size']) <= 1_450_000_000_000:
            raise ValueError('device size differs from the r7gd.12xlarge disk layout')
        if disk.get('children') or disk.get('fstype') or any(disk.get('mountpoints') or []):
            raise ValueError('instance-store disk is partitioned, formatted or mounted')
        if signatures[name].get('signatures'):
            raise ValueError('instance-store disk has an existing signature')
        selected.append(disk)
    if len(selected) != 2 or len({d['path'] for d in selected}) != 2 or len({d['serial'] for d in selected}) != 2:
        raise ValueError('expected exactly two distinct blank instance-store disks')
    return sorted(selected, key=lambda d: d['serial'])


def snapshot():
    return json.loads(command(['lsblk', '--json', '--bytes', '--paths', '--output',
        'PATH,TYPE,MODEL,SERIAL,SIZE,FSTYPE,MOUNTPOINTS']))


def prepare(receipt):
    if os.geteuid() != 0:
        raise ValueError('storage setup requires root on the disposable host')
    if receipt.exists() or MOUNT.exists():
        raise ValueError('native storage setup is exclusive; existing state is never reformatted')
    topology = snapshot()
    candidates = [d for d in topology['blockdevices'] if str(d.get('model', '')).strip() == MODEL]
    signatures = {d['path']: json.loads(command(['wipefs', '--json', '--no-act', d['path']])) for d in candidates}
    disks = plan(topology, signatures)
    # Re-read before any destructive command. No EBS device is ever in this list.
    if snapshot() != topology:
        raise ValueError('block topology changed during storage admission')
    MOUNT.mkdir()
    rows = []
    for i, disk in enumerate(disks):
        device = disk['path']; destination = MOUNT / f'data{i}'
        subprocess.run(['mkfs.ext4', '-q', '-m', '0', '-L', f'sparq-native-{i}', device], check=True, timeout=300)
        destination.mkdir()
        subprocess.run(['mount', '-o', 'noatime', '--', device, str(destination)], check=True, timeout=60)
        subprocess.run(['chown', 'ubuntu:ubuntu', str(destination)], check=True, timeout=30)
        rows.append(dict(device=device, serial=disk['serial'], model=disk['model'],
                         bytes=disk['size'], mount=str(destination), filesystem='ext4', options='noatime'))
    receipt.parent.mkdir(parents=True, exist_ok=True)
    receipt.write_text(json.dumps(dict(scope='dedicated disposable EC2 instance-store only',
        layout='two independent filesystems; no RAID', before=topology,
        after=snapshot(), mounts=rows, instance_store_durable_across_host_loss=False), indent=2) + '\n')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--receipt', type=Path, required=True)
    prepare(parser.parse_args().receipt)
