import pathlib,types,sys,unittest,io,json
p=pathlib.Path(__file__).resolve().parent
source=pathlib.Path('/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue6480/scripts/tests/test_js_wasm_pack_install.py')
text=source.read_text()
mutations=[('ignore-cwd','self.assertEqual(cwd, ["working-directory: ."])','pass # control: omit root check','test_root_install_wrong_or_missing_directory_is_rejected'),('ignore-order','min(npm_ci_at),','install_at[0] + 1,','test_npm_ci_before_wasm_pack_is_rejected'),('accept-npm-install','lines[0] != "npm ci"','lines[0] not in ("npm ci", "npm install")','test_root_install_missing_or_wrong_command_is_rejected'),('inline-only','if not lines or lines[0] != "npm ci":','if not any(re.match(r"^\\s*run:\\s*npm ci\\s*$", line) for line in block) or not lines or lines[0] != "npm ci":','test_root_install_inline_and_block_forms')]
results=[]
for name,old,new,test in mutations:
 assert text.count(old)==1,(name,text.count(old))
 changed=text.replace(old,new);(p/(name+'.py')).write_text(changed)
 module=types.ModuleType('control_'+name.replace('-','_'));module.__file__=str(source);sys.modules[module.__name__]=module
 exec(compile(changed,str(source),'exec'),module.__dict__)
 stream=io.StringIO();suite=unittest.defaultTestLoader.loadTestsFromName('JsLaneWasmPackInstall.'+test,module)
 result=unittest.TextTestRunner(stream=stream,verbosity=2).run(suite)
 (p/(name+'.log')).write_text(stream.getvalue())
 assert result.testsRun==1 and result.failures and not result.errors,(name,stream.getvalue())
 results.append({'name':name,'tests':result.testsRun,'failures':len(result.failures),'errors':len(result.errors),'killed':True,'test':test})
(p/'controls.json').write_text(json.dumps(results,indent=2)+'\n');print(json.dumps(results,indent=2))
