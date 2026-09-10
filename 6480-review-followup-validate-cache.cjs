const fs=require('fs'), path=require('path');
const root=__dirname, npmRoot='/private/tmp/sparq-pr6049/.throughput-monitor/direct-6480/implementation/tools/npm-11.17.0';
const cacache=require(npmRoot+'/node_modules/cacache'), semver=require(npmRoot+'/node_modules/semver');
const lock=JSON.parse(fs.readFileSync(process.argv[2])).packages, changes=JSON.parse(fs.readFileSync(root+'/targeted-update-changes.json')).changed;
(async()=>{const entries=await cacache.ls(root+'/cache/_cacache');const manifests=new Map();
for(const key of Object.keys(entries)){try { const got=await cacache.get(root+'/cache/_cacache',key); const j=JSON.parse(got.data); if(j.name&&j.versions)manifests.set(j.name,j); } catch(e){} }
const rows=[];
for(const k of changes){const e=lock[k];const name=k.slice('node_modules/'.length);const data=manifests.get(name);if(!data||!data.versions[e.version])throw new Error('missing exact cached manifest '+name+' '+e.version);
const m=data.versions[e.version]; if(e.resolved!==m.dist.tarball||e.integrity!==m.dist.integrity)throw new Error('integrity/URL differs '+k);
for(const f of ['dependencies','optionalDependencies','engines','os','cpu','libc']){const a=JSON.stringify(e[f]||null,Object.keys(e[f]||{}).sort());const b=JSON.stringify(m[f]||null,Object.keys(m[f]||{}).sort()); if(a!==b)throw new Error('manifest field differs '+k+' '+f);}
fs.mkdirSync(root+'/resolved-metadata',{recursive:true});fs.writeFileSync(root+'/resolved-metadata/'+name.replaceAll('/','__')+'.json',JSON.stringify(m,null,2)+'\n');
rows.push({path:k,version:e.version,metadata_integrity_matches:true,manifest_fields_match:true});}
const sharp=lock['node_modules/sharp'];if(!semver.satisfies(sharp.version,'^0.34.3 || ^0.35.3')||!semver.satisfies(process.version,sharp.engines.node)||!semver.satisfies('22.0.0',sharp.engines.node))throw new Error('engines/range');
console.log(JSON.stringify({node:process.version,sharp:sharp.version,nextRangeCompatible:true,node22RangeCompatible:true,checked:rows},null,2));})().catch(e=>{console.error(e.stack);process.exit(1)});
