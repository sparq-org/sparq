from pathlib import Path
import json,subprocess,sys,time
p=Path(__file__).resolve().parent
records=json.loads((p/'requests.json').read_text()) if (p/'requests.json').exists() else []
assert not (p/'read-error.json').exists(), 'prior actual request failure; do not retry'
assert len(records)<20,'20 logical read ceiling'
name,endpoint=sys.argv[1:]
argv=['/opt/homebrew/bin/gh','api',endpoint]
t=time.monotonic();c=subprocess.run(argv,capture_output=True,timeout=50)
(p/name).write_bytes(c.stdout);(p/(name+'.stderr')).write_bytes(c.stderr)
record={'ordinal':len(records)+1,'argv':argv,'output':name,'exit':c.returncode,'seconds':time.monotonic()-t};records.append(record)
(p/'requests.json').write_text(json.dumps(records,indent=2)+'\n')
print(json.dumps(record))
if c.returncode:
 (p/'read-error.json').write_text(json.dumps(record));print(c.stderr.decode());sys.exit(1)
if endpoint=='rate_limit':
 v=json.loads(c.stdout)['resources']['core'];print(json.dumps(v));assert v['remaining']>0,'budget exhausted'
