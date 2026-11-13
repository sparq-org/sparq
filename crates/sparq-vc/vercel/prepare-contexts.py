#!/usr/bin/env python3
"""Fetch immutable request-time cache inputs BEFORE starting/building the service."""
import hashlib
import json
from pathlib import Path
import sys
import urllib.request

CONTEXTS = {
    'credentials-v2.jsonld': 'https://www.w3.org/ns/credentials/v2',
    'credentials-v1.jsonld': 'https://www.w3.org/2018/credentials/v1',
    'examples-v2.jsonld': 'https://www.w3.org/ns/credentials/examples/v2',
    'data-integrity-v2.jsonld': 'https://w3id.org/security/data-integrity/v2',
    'data-integrity-v1.jsonld': 'https://w3id.org/security/data-integrity/v1',
    'multikey-v1.jsonld': 'https://w3id.org/security/multikey/v1',
    'did-v1.jsonld': 'https://www.w3.org/ns/did/v1',
}

def main():
    directory = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).parent / 'contexts'
    directory.mkdir(parents=True, exist_ok=True)
    manifest = {}
    # Collect all documents before replacing the current cache.
    documents = {}
    for filename, url in CONTEXTS.items():
        request = urllib.request.Request(url, headers={'Accept': 'application/ld+json, application/json'})
        with urllib.request.urlopen(request, timeout=60) as response:
            data = response.read(2 * 1024 * 1024 + 1)
            if len(data) > 2 * 1024 * 1024:
                raise ValueError(f'context is too large: {url}')
            value = json.loads(data)
            if not isinstance(value, dict) or '@context' not in value:
                raise ValueError(f'not a JSON-LD context: {url}')
            documents[filename] = data
            manifest[filename] = {'url': url, 'resolvedUrl': response.url,
                                  'sha256': hashlib.sha256(data).hexdigest()}
    for filename, data in documents.items():
        (directory / filename).write_bytes(data)
    (directory / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    print(f'Prepared offline cache in {directory}')

if __name__ == '__main__':
    main()
