// [GPT-6 Astra] One isolated input; result collection is not an invariance assertion.
const fs=require('node:fs');
const {explore,LIMITS}=require('./explore.cjs');
const [file,hash='SHA256',control='candidate']=process.argv.slice(2);
const input=fs.readFileSync(file,'utf8');
const Baseline=require('../reference-v5/reference/package/lib/RDFC10Sync');
const NQuads=require('../reference-v5/reference/package/lib/NQuads');
let baseline;
try {const map=new Map();const output=new Baseline({canonicalIdMap:map,messageDigestAlgorithm:hash,maxDeepIterations:4000,timeout:1000}).main(NQuads.parse(input));baseline={status:'complete',output,map:Object.fromEntries(map)};}
catch(e){baseline={status:'error',error:e.message};}
console.log(JSON.stringify({limits:LIMITS,baseline,result:explore(input,{hash,disableRecursive:control==='disable-recursive',firstOnly:control==='first-only'})}));
