#!/usr/bin/env python3
"""Run the draft envelope source under the programme's exact native toolchain."""
import argparse
import json
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[3]
HERE = Path(__file__).resolve().parent


FIELD = {'kind': 'field'}
BOOL = {'kind': 'boolean'}
DIGEST = {'kind': 'array', 'length': 2,
          'type': {'kind': 'integer', 'sign': 'unsigned', 'width': 128}}
PUBLIC_INPUTS = {
    'issuer': [('trusted_root', FIELD)],
    'holder': [('request_digest', DIGEST)],
    'status': [('accepted_root', FIELD), ('allow_missing', BOOL)],
    'validity': [('now', {'kind': 'integer', 'sign': 'unsigned', 'width': 64})],
    'dataset': [('request_digest', DIGEST)],
    'request': [('request_digest', DIGEST)],
}


def check_public_abi(name, abi):
    """Check every public parameter's name, order, type, width and array length."""
    actual = [(p['name'], p['type']) for p in abi['parameters'] if p['visibility'] == 'public']
    if actual != PUBLIC_INPUTS[name] or abi['return_type'] is not None:
        raise ValueError(f'{name}: public-input contract differs: {actual!r}')
    return [name for name, _ in actual]


def run(args, cwd, **kwargs):
    return subprocess.run(args, cwd=cwd, check=True, text=True, **kwargs)


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
    with tempfile.TemporaryDirectory(prefix='lane-e-envelope-') as tmp:
        work = Path(tmp)
        core = work / 'core'
        (core / 'src').mkdir(parents=True)
        (core / 'src/lib.nr').write_text('pub mod table;\npub mod envelope;\n')
        # Same source bytes, isolated manifest. The lane B prerelease version
        # requirement is not accepted by the pinned nargo manifest parser.
        for name in ('table.nr', 'envelope.nr'):
            source = ROOT / 'zk/noir-query/core/src' / name
            target = core / 'src' / name
            shutil.copyfile(source, target)
            assert source.read_bytes() == target.read_bytes()
        compose = ROOT / 'zk/compose/compose_core'
        (core / 'Nargo.toml').write_text(
            '[package]\nname = "envelope_core_fixture"\ntype = "lib"\nauthors = ["sparq"]\n'
            f'\n[dependencies]\ncompose_core = {{ path = "{compose}" }}\n')
        packages = ['tests'] + (['issuer', 'holder', 'status', 'validity', 'dataset', 'request'] if args.measure else [])
        for name in packages:
            package = work / name
            (package / 'src').mkdir(parents=True)
            shutil.copyfile(HERE / name / 'src/main.nr', package / 'src/main.nr')
            (package / 'Nargo.toml').write_text(
                f'[package]\nname = "envelope_{name}"\ntype = "bin"\nauthors = ["sparq"]\n'
                f'\n[dependencies]\nenvelope_core = {{ path = "../core" }}\ncompose_core = {{ path = "{compose}" }}\n')
        run(['nargo', 'test', '--test-threads', '1', '--deny-warnings'], work / 'tests')
        if args.measure:
            measurements = {}
            for name in packages[1:]:
                package = work / name
                run(['nargo', 'compile', '--deny-warnings'], package)
                artifact = package / f'target/envelope_{name}.json'
                abi = json.loads(artifact.read_text())['abi']
                public = check_public_abi(name, abi)
                result = run(['bb', 'gates', '-s', 'ultra_honk', '-b', str(artifact)], package, capture_output=True)
                measurements[name] = {'public_inputs': public, 'backend': json.loads(result.stdout)}
                print(f'Measured {name}', flush=True)
            args.output.write_text(json.dumps({'nargo': nargo, 'bb': bb, 'checks': measurements}, indent=2) + '\n')


if __name__ == '__main__':
    main()
