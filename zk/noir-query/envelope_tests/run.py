#!/usr/bin/env python3
"""Run the draft envelope source under the programme's exact native toolchain."""
import argparse
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[3]
HERE = Path(__file__).resolve().parent


FIELD = {'kind': 'field'}
BOOL = {'kind': 'boolean'}
U32 = {'kind': 'integer', 'sign': 'unsigned', 'width': 32}
# Request and authenticated credential instants are exact Unix epoch milliseconds.
U64 = {'kind': 'integer', 'sign': 'unsigned', 'width': 64}
DIGEST = {'kind': 'array', 'length': 32,
          'type': {'kind': 'integer', 'sign': 'unsigned', 'width': 8}}
POLICY_INPUTS = [
    ('issuer_root', FIELD), ('issuer_depth', U32),
    ('holder_required', BOOL), ('holder_issuers_present', BOOL),
    ('holder_issuer_count', U32),
    ('holder_issuers', {'kind': 'array', 'length': 4, 'type': FIELD}),
    ('status_required', BOOL), ('allow_missing_status', BOOL),
    ('snapshot_root', FIELD), ('snapshot_depth', U32),
    ('validity_required', BOOL), ('now', U64),
    ('freshness_required', BOOL), ('earliest_created', U64),
]
PUBLIC_INPUTS = {
    name: [('request_digest', DIGEST)] + POLICY_INPUTS
    for name in ('issuer', 'holder', 'status', 'validity', 'freshness', 'dataset', 'request', 'total')
}


def check_public_abi(name, abi):
    """Check every public parameter's name, order, type, width and array length."""
    actual = [(p['name'], p['type']) for p in abi['parameters'] if p['visibility'] == 'public']
    if actual != PUBLIC_INPUTS[name] or abi['return_type'] is not None:
        raise ValueError(f'{name}: public-input contract differs: {actual!r}')
    return [name for name, _ in actual]


def run(args, cwd, **kwargs):
    return subprocess.run(args, cwd=cwd, check=True, text=True, **kwargs)


def run_native_tests():
    """Discover every case, then serialize the large credential batches."""
    package = HERE / 'tests'
    discovered = run(['nargo', 'test', '--list-tests', '--deny-warnings'],
                     package, capture_output=True).stdout.splitlines()
    names = []
    for line in discovered:
        parts = line.split()
        if len(parts) != 2 or parts[0] != 'envelope_tests':
            raise ValueError(f'unexpected native test discovery: {line!r}')
        names.append(parts[1])
    if not names or len(names) != len(set(names)):
        raise ValueError('native test discovery must be nonempty and unique')
    large = [name for name in names if name.startswith('four')]
    regular = [name for name in names if name not in large]
    if 'four_credentials_enforce_all_conditions_and_merge' not in large:
        raise ValueError('signed Solid case missing from native discovery')
    batches = [(regular, '4')] + [([name], '1') for name in large]
    scheduled = [name for batch, _ in batches for name in batch]
    if len(scheduled) != len(names) or set(scheduled) != set(names):
        raise ValueError('native partitions must execute every discovered case once')
    failed = []
    for batch, threads in batches:
        command = ['nargo', 'test', '--test-threads', threads, '--deny-warnings',
                   '--exact', *batch]
        result = subprocess.run(command, cwd=package, check=False)
        if result.returncode:
            failed.append((command, result.returncode))
    if failed:
        command, code = failed[0]
        raise subprocess.CalledProcessError(code, command)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--measure', action='store_true')
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    if args.measure and args.output is None:
        parser.error("--measure requires --output for private measurement data")
    nargo = run(['nargo', '--version'], ROOT, capture_output=True).stdout.splitlines()[0]
    bb = run(['bb', '--version'], ROOT, capture_output=True).stdout.strip()
    if nargo != 'nargo version = 1.0.0-beta.21' or bb != '5.0.0-nightly.20260324':
        raise SystemExit('programme toolchain pin mismatch')
    packages = ['tests'] + (['issuer', 'holder', 'status', 'validity', 'freshness', 'dataset', 'request', 'total'] if args.measure else [])
    run_native_tests()
    if args.measure:
        measurements = {}
        for name in packages[1:]:
            package = HERE / name
            run(['nargo', 'compile', '--deny-warnings'], package)
            artifact = package / f'target/envelope_{name}.json'
            abi = json.loads(artifact.read_text())['abi']
            public = check_public_abi(name, abi)
            result = run(['bb', 'gates', '-s', 'ultra_honk', '-t', 'noir-recursive', '-b', str(artifact)], package, capture_output=True)
            measurements[name] = {'public_inputs': public, 'backend': json.loads(result.stdout)}
            print(f'Measured {name}', flush=True)
        args.output.write_text(json.dumps({'nargo': nargo, 'bb': bb, 'checks': measurements}, indent=2) + '\n')


if __name__ == '__main__':
    main()
