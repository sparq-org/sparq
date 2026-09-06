# Disposable ARM build host

<!-- [GPT-6] Reference for the explicit build/test lane; no benchmark admission. -->

`build-host.sh` starts the constrained `build` mode of the existing EC2 launcher.
It reuses the exact study tags, protected-instance exclusions, source bundle,
private SSH ingress, result verification and cleanup. It starts `build-jobs.py`,
which waits for explicitly submitted jobs. It never invokes the experiment runner.
The current public AWS CSV is checked by `build-cost.py` before creating resources;
the existing study allocation must include its already reserved contingency.

The coordinator runs the launcher in a persistent local terminal after reviewing
the concrete source and local tests:

```sh
SPARQ_POD_PRIOR_AWS_USD='<recorded prior study allocation>' \
SPARQ_POD_RESULTS_LOCAL='<absolute artifact directory>' \
bash bench/ac/million/build-host.sh
```

The private local `build-host.json` contains the host IP, key **path**, known-hosts
path, source commit and absolute deadline. Key contents must not be printed or
copied into artifacts. Wait for the retrieved `READY` marker before submission.
Cleanup invalidates the connection paths and removes the temporary key.

Commit each source snapshot in its isolated worktree. A job file contains an array
of explicit argv arrays, for example:

```json
[["cargo", "test", "--locked", "-p", "sparq-core"]]
```

Submit a uniquely named, bounded job with:

```sh
python3 bench/ac/million/build-jobs.py submit \
  --host '<artifact directory>/build-host.json' \
  --source '<absolute clean source worktree>' \
  --id '<unique-lowercase-job-name>' --timeout 3600 \
  --commands '<absolute commands.json>'
```

The submitter hashes a complete committed source archive; the host verifies it
before extraction. Commands run serially against a shared Cargo target directory,
with the repository's pinned Rust toolchain. The host service enforces aggregate
memory and no-swap bounds. Source admission and in-flight disk floors, heartbeat
and deadline checks are enforced by the job runner. Named Python test files and
locked Cargo build, test, check, clippy and doc commands are accepted; rustdoc
warnings are errors. Benchmark and arbitrary shell command dispatch are rejected.
Conformance and performance executables invoked through `cargo run` remain PR-CI
gates; this host's narrower dispatch is not a replacement for the full merge gate.

Each completed job publishes its command log hashes, source identity, lockfile and
toolchain hashes, command outcomes and stop reason. A passed build is functional
evidence only. The collector retrieves these small results, not the shared Cargo
target. Each job's source archive and extracted tree are removed after success or
failure; the logs and source identity remain available for diagnosis.

After all desired jobs finish, the coordinator requests finalization:

```sh
python3 bench/ac/million/build-jobs.py finish \
  --host '<artifact directory>/build-host.json'
```

The host publishes the final manifest before `DONE`. The launcher verifies every
manifest entry before successful completion, then terminates the exact study host
and deletes its disposable network/key resources. The remote absolute watchdog
and independently running local supervisor enforce the deadline even if the
launcher's terminal is lost. The supervisor and normal cleanup confirm termination;
missing or mismatched identity tags fail closed.
