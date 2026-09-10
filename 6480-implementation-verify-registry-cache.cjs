// GPT-6 Astra: read npm's verified response cache; no network or package execution.
const fs = require('fs');
const path = require('path');
const root = __dirname;
const cacache = require(path.join(root, 'tools/npm-11.17.0/node_modules/cacache'));
const lock = JSON.parse(fs.readFileSync(path.join(JSON.parse(fs.readFileSync(path.join(root, 'protocol.json'))).worktree, 'package-lock.json')));
(async () => {
  const entries = await cacache.ls(path.join(root, 'cache/_cacache'));
  const names = ['next', '@next/env', ...Object.keys(lock.packages['node_modules/next'].optionalDependencies).filter(x => x.startsWith('@next/swc-'))];
  const out = [];
  for (const name of names) {
    const url = 'https://registry.npmjs.org/' + name.replace('/', '%2f');
    const matching = Object.keys(entries).filter(x => x === 'make-fetch-happen:request-cache:' + url);
    if (matching.length !== 1) throw Error('Unexpected cache identity for ' + name);
    const result = await cacache.get(path.join(root, 'cache/_cacache'), matching[0]);
    const version = JSON.parse(result.data).versions['15.5.24'];
    if (!version || version.name !== name || version.version !== '15.5.24') throw Error('Missing pinned registry version');
    const item = lock.packages['node_modules/' + name];
    if (item.resolved !== version.dist.tarball || item.integrity !== version.dist.integrity) throw Error('Registry/lock dist mismatch for ' + name);
    out.push({name, registry_url: url, response_cache_integrity: result.integrity, response_bytes: result.data.length, lock_dist_match: true, version_metadata: version});
  }
  fs.writeFileSync(path.join(root, 'registry-versions.json'), JSON.stringify(out, null, 2) + '\n');
  console.log(JSON.stringify({verified_registry_version_records: out.length, packages: names, network_requests: 0}));
})().catch(error => { console.error(error); process.exitCode = 1; });
