#!/usr/bin/env python3
"""[GPT-6] Extract a finalized native campaign; produce no substitute cached results."""
import argparse
import json
from pathlib import Path
import shutil
import tempfile
from native_analysis import analyze


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('artifacts', type=Path)
    parser.add_argument('--review', type=Path, help='Review bound to source, binary, campaign and final manifest hashes')
    parser.add_argument('--scratch-directory', type=Path)
    parser.add_argument('--source-root', type=Path)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    scratch = args.scratch_directory or Path(tempfile.gettempdir())
    if shutil.disk_usage(scratch).free < 1024**3: raise RuntimeError('analysis requires a1GiB scratch reserve')
    options = {'review_path': args.review, 'scratch_root': scratch}
    if args.source_root is not None: options['source_root'] = args.source_root
    result = analyze(args.artifacts, **options)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2, sort_keys=True, allow_nan=False) + '\n')
    print(json.dumps({'output': str(args.output), 'analysis_kind': result['analysis_kind'],
                      'integrity': result['artifact_integrity']['complete'], 'source_review': result['source_review']['status'],
                      'eligible_cells': len(result['headline_eligible_cells'])}))


if __name__ == '__main__': main()
