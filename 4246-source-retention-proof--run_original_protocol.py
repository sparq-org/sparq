from pathlib import Path
import importlib.util,sys,os,json,hashlib
w=Path('/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue4246')
source=w/'scripts/feature_off_autodeclare.py'
spec=importlib.util.spec_from_file_location('original_feature_off_autodeclare',source);module=importlib.util.module_from_spec(spec);sys.modules[spec.name]=module;spec.loader.exec_module(module)
# Execute the original CLI/protocol. The only wrapper is an external cargo argv
# shim adding --locked and recording actual compiler output. No builder/result
# function is replaced, injected, mocked or short-circuited.
raise SystemExit(module.main(['--repo',str(w),'--base-sha','a42a9e89dec485f6a319c47cb3635c59cb5a2270','--head-sha','6334b338587fe5c635c69a09e134917ec35eaaca','--pr','6469','--report-only','--summary-file',str(Path(os.environ['FEATOFF_PROOF_EVIDENCE'])/'report-only-summary.md')]))
