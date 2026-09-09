// [GPT-6 Astra] Unmodified pinned synchronous reference; fixed per-case options.
const fs = require('node:fs');
const RDFC10Sync = require('./reference/package/lib/RDFC10Sync');
const NQuads = require('./reference/package/lib/NQuads');
const [file, mode, hash, budget] = process.argv.slice(2);
const map = new Map();
const options = {canonicalIdMap:map, messageDigestAlgorithm:hash, timeout:1000};
if(budget !== 'default') options.maxDeepIterations = Number(budget);
try {
  const value = new RDFC10Sync(options).main(NQuads.parse(fs.readFileSync(file, 'utf8')));
  console.log(JSON.stringify({value:mode==='map'?Object.fromEntries(map):value}));
} catch(e) {console.log(JSON.stringify({error:e.message, error_debug:e.name}));}
