// GPT-6 Astra: metadata compatibility only, not runtime/build validation.
const fs=require('fs'),path=require('path');
const p=__dirname,proto=JSON.parse(fs.readFileSync(path.join(p,'protocol.json')));
const semver=require(path.join(p,'tools/npm-11.17.0/node_modules/semver'));
const lock=JSON.parse(fs.readFileSync(path.join(proto.worktree,'package-lock.json')));
const next=lock.packages['node_modules/next'];
const checks={local_node:semver.satisfies(process.versions.node,next.engines.node),ci_node22_minimum:semver.satisfies('22.0.0',next.engines.node),react:semver.satisfies(lock.packages['node_modules/react'].version,next.peerDependencies.react),react_dom:semver.satisfies(lock.packages['node_modules/react-dom'].version,next.peerDependencies['react-dom'])};
if(Object.values(checks).some(x=>!x))process.exit(2);
console.log(JSON.stringify({node:process.version,checks,qualification:'Published semver ranges only; Node22 install/runtime and app compatibility not executed.'}));
