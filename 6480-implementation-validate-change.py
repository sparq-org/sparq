# GPT-6 Astra: exact three-file dependency scope and platform completeness checks.
from pathlib import Path
import copy,hashlib,json,subprocess
w=Path.cwd();e=Path(__file__).resolve().parent;p=json.loads((e/'protocol.json').read_text());base=p['base'];sha=lambda f:hashlib.sha256(f.read_bytes()).hexdigest()
old=json.loads((e/'before/package-lock.json').read_text());new=json.loads((w/'package-lock.json').read_text())
registry={x['name']:x['version_metadata'] for x in json.loads((e/'registry-versions.json').read_text())}
family=['next','@next/env']+[k for k in registry if k.startswith('@next/swc-')]
assert len(family)==10 and len(set(family))==10
expected=copy.deepcopy(old)
for key in ['site','gui/app']:expected['packages'][key]['dependencies']['next']='^15.5.24'
for name in family:
 key='node_modules/'+name;data=registry[name];item=expected['packages'][key]
 item.update(version='15.5.24',resolved=data['dist']['tarball'],integrity=data['dist']['integrity'])
 if name=='next':
  item['dependencies']['@next/env']='15.5.24'
  for swc in family:
   if swc.startswith('@next/swc-'):item['optionalDependencies'][swc]='15.5.24'
  item['optionalDependencies']['sharp']=data['optionalDependencies']['sharp']
assert new==expected,'Unexpected full-lock diff beyond the approved package fields'
assert new['packages']['node_modules/next']['dependencies']==registry['next']['dependencies']
assert new['packages']['node_modules/next']['optionalDependencies']==registry['next']['optionalDependencies']
assert new['packages']['node_modules/next']['engines']==registry['next']['engines']
assert new['packages']['node_modules/next']['peerDependencies']==registry['next']['peerDependencies']
for key in ['site','gui/app']:
 x=json.loads((e/'before'/key/'package.json').read_text());x['dependencies']['next']='^15.5.24'
 assert json.loads((w/key/'package.json').read_text())==x
swc=[k for k in new['packages'] if k.startswith('node_modules/@next/swc-')];assert len(swc)==8
libc={k:new['packages'][k]['libc'] for k in swc if '/swc-linux-' in k};assert len(libc)==4
assert all(new['packages'][k]['libc']==old['packages'][k]['libc'] for k in libc)
assert set(old['packages'])==set(new['packages'])
changed=[k for k in old['packages'] if old['packages'][k]!=new['packages'][k]];assert len(changed)==12
preserved={}
for path in ['package.json','site/next.config.ts','gui/app/next.config.ts','site/scripts/vr.sh']:
 raw=subprocess.check_output(['git','show',base+':'+path]);assert raw==(w/path).read_bytes();preserved[path]=sha(w/path)
paths=['gui/app/package.json','package-lock.json','site/package.json'];assert sorted(subprocess.check_output(['git','diff','--name-only',base],text=True).splitlines())==paths
assert subprocess.run(['git','diff','--check'],capture_output=True).returncode==0
assert all(not (w/k).exists() for k in ['node_modules','site/node_modules','gui/app/node_modules'])
locks=subprocess.check_output(['git','ls-files','*package-lock.json'],text=True).splitlines();copies=[]
for lock in locks:
 j=json.loads((w/lock).read_text())
 for key,value in j.get('packages',{}).items():
  if key.endswith('/next'):copies.append({'lock':lock,'key':key,'version':value['version']})
assert copies==[{'lock':'package-lock.json','key':'node_modules/next','version':'15.5.24'}]
# Tiny in-memory negative controls verify this exact expected-diff comparison rejects
# accidental old version, missing native platform, and Linux selector removal.
controls=[]
for name in ['old-next','missing-swc','missing-libc']:
 bad=copy.deepcopy(new)
 if name=='old-next':bad['packages']['node_modules/next']['version']='15.5.21'
 if name=='missing-swc':del bad['packages']['node_modules/@next/swc-win32-arm64-msvc']
 if name=='missing-libc':del bad['packages']['node_modules/@next/swc-linux-x64-musl']['libc']
 assert bad!=expected;controls.append({'fixture':name,'rejected_by_full_lock_equality':True})
result={'full_lock_equals_expected_publisher_update':True,'package_entries_before_after':[len(old['packages']),len(new['packages'])],'changed_package_entries':changed,'package_entries_added_removed':[0,0],'next_copies_all_tracked_locks':copies,'swc_count':len(swc),'linux_libc_preserved':libc,'root_overrides_and_configs_preserved_sha256':preserved,'sharp_locked_version':new['packages']['node_modules/sharp']['version'],'sharp_range_before_after':[old['packages']['node_modules/next']['optionalDependencies']['sharp'],new['packages']['node_modules/next']['optionalDependencies']['sharp']],'unrelated_entries_identical':len(new['packages'])-len(changed),'manifest_minima':['site:^15.5.24','gui/app:^15.5.24'],'node_modules_absent':True,'diff_check_exit':0,'metadata_only_negative_controls':controls}
(e/'validation.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result,indent=2))
