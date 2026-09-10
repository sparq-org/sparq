// [GPT-6 Astra] Experimental complete-outcome search; never production or cache code.
const Candidate = require('./candidate/lib/RDFC10Sync');
const NQuads = require('./candidate/lib/NQuads');
const LIMITS = Object.freeze({hndqCalls:4000, executions:64, milliseconds:1000});
class Choice extends Error {constructor(count, context){super('choice');this.count=count;this.context=context;}}
class Exhausted extends Error {}
function reconstruct(quads, map) {
  const term = x => x.termType==='BlankNode'?{...x,value:map[x.value]}:x;
  return NQuads.serialize(quads.map(q=>({...q,subject:term(q.subject),object:term(q.object),graph:term(q.graph)})));
}
function explore(input, {hash='SHA256', disableRecursive=false, firstOnly=false}={}) {
  const quads=NQuads.parse(input), pending=[[]], started=Date.now();
  const stats={executions:0,hndqCalls:0,leaves:0,choices:0,topComplete:0,topPartial:0,topCrossGroup:0,topOverlapping:0,recursive:0,maxPending:1};
  let best=null;const distinct=new Set();
  try {
    while(pending.length) {
      if(stats.executions>=LIMITS.executions) throw new Exhausted('execution budget');
      const prefix=pending.pop();let position=0;
      const map=new Map(), instance=new Candidate({canonicalIdMap:map,messageDigestAlgorithm:hash,maxDeepIterations:4000,timeout:1000});
      ++stats.executions;
      instance.experimentalTick=()=>{
        if(++stats.hndqCalls>LIMITS.hndqCalls) throw new Exhausted('global HNDQ budget');
        if(Date.now()-started>LIMITS.milliseconds) throw new Exhausted('total deadline');
      };
      instance.chooseEqual=(options,context)=>{
        if(options.length===0) throw new Error('empty choice');
        if(options.length===1 || firstOnly || (disableRecursive&&context.stage==='recursive')) return options[0];
        if(position===prefix.length) throw new Choice(options.length,context);
        const chosen=prefix[position++];
        if(chosen>=options.length) throw new Error('invalid replay prefix');
        return options[chosen];
      };
      try {
        const output=instance.main(quads);
        if(position!==prefix.length) throw new Error('unused replay prefix');
        const issuer=Object.fromEntries(map);
        if(reconstruct(quads,issuer)!==output) throw new Error('output/map mismatch');
        ++stats.leaves;distinct.add(output);
        if(best===null || Buffer.compare(Buffer.from(output),Buffer.from(best.output))<0) best={output,map:issuer};
      } catch(error) {
        if(!(error instanceof Choice)) throw error;
        ++stats.choices;
        const c=error.context;
        if(c.stage==='recursive')++stats.recursive;
        if(c.stage==='top') {if(c.complete)++stats.topComplete;if(c.partial)++stats.topPartial;if(c.crossGroup)++stats.topCrossGroup;if(c.overlap)++stats.topOverlapping;}
        if(pending.length+error.count>LIMITS.executions) throw new Exhausted('pending execution budget');
        for(let i=error.count-1;i>=0;--i)pending.push([...prefix,i]);
        stats.maxPending=Math.max(stats.maxPending,pending.length);
      }
      if(Date.now()-started>LIMITS.milliseconds)throw new Exhausted('total deadline');
    }
    return {status:'complete',...best,distinctOutcomes:distinct.size,mapConsistent:true,stats};
  } catch(error) {
    // Never return a minimum from an incomplete search, even when a leaf exists.
    return {status:'error',error:error.message,kind:error instanceof Exhausted?'experimental_budget':error.name,stats,partialOutcomes:distinct.size};
  }
}
module.exports={explore,LIMITS};
