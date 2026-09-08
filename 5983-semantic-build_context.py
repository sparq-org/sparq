"""[GPT-6 Astra] Reproducible frozen-source excerpt assembly; no source mutation."""
from pathlib import Path
import subprocess,json,re
out=Path(__file__).parent
git=lambda *args:subprocess.check_output(['git',*args],text=True)
head=git('rev-parse','HEAD').strip(); assert head=='9e8bdfc95c916b62550fb8c3142f804bbbfb2444'
files={}; contexts=[]
def source(path):
 if path not in files:files[path]=git('show',head+':'+path)
 return files[path]
def excerpt(path,label,start,end):
 body=''.join(source(path).splitlines(keepends=True)[start-1:end])
 contexts.append(dict(path=path,label=label,start=start,end=end,source_blob=git('rev-parse',head+':'+path).strip(),body=body))
def item(path,name,kind='fn',indent=0,public=False):
 s=source(path);pad=' '*indent;visibility=r'pub ' if public else r'(?:pub(?:\([^)]*\))? )?'
 ms=list(re.finditer(r'^'+pad+visibility+r'(?:async )?'+kind+' '+re.escape(name)+r'\b',s,re.M));assert len(ms)==1,(path,name,len(ms))
 m=ms[0];end=re.search(r'^'+pad+r'}',s[m.end():],re.M);assert end,name
 excerpt(path,name,s.count('\n',0,m.start())+1,s.count('\n',0,m.end()+end.end())+1)
e='crates/sparq-engine/src/exec.rs';l='crates/sparq-engine/src/lib.rs';c='crates/sparq-core/src/store.rs';d='crates/sparq-core/src/dict.rs'
for name in ['eval_select','eval_modified','try_topk_orderby','has_intra_triple_repeated_var','is_conjunctive','flatten_conjunction','collect_vars','prepare_pattern','scan_to_bindings','eval_graph_named','eval_graph_named_pref','order_bindings']:item(e,name)
s=source(e);excerpt(e,'budget production portion (unchanged test modules omitted)',s[:s.index('pub(crate) mod budget {')].count('\n')+1,s[:s.index('    /// [SONNET-4.6] (sq-qk6ac) Direct tests')].count('\n'))
item(e,'view','mod')
for name,kind in [('SortCell','enum'),('sort_cell_val','fn'),('cmp_sort_cells','fn'),('cmp_sort_num','fn'),('cmp_exact_lex_f64','fn'),('sort_cell_term','fn'),('str_id_value','fn'),('cmp_strid_val','fn'),('compare_values','fn')]:item(e,name,kind)
for name in ['with_view','query_view','query_view_with_budget','active_dataset','view_scope','query','query_with_budget','query_prepared_with_budget']:item(l,name)
for name in ['QueryBudget','DatasetView']:item(l,name,'struct')
item(l,'DefaultGraphMode','enum');s=source(l);m=re.search(r'^impl QueryBudget \{',s,re.M);end=re.search(r'^}',s[m.end():],re.M);excerpt(l,'QueryBudget methods',s.count('\n',0,m.start())+1,s.count('\n',0,m.end()+end.end())+1)
item('crates/sparq-engine/src/dataset.rs','build_active')
for name in ['explain_analyze','explain_analyze_with_budget']:item('crates/sparq-engine/src/explain.rs',name)
excerpt(c,'Perm and BUILT compile-time inventory',1,56)
for name in ['choose','choose_sorted','scan_sorted','bounds','scan_with','rows_in','merge','count_correction','to_spo']:item(c,name,indent=4)
item(c,'Scan','struct');excerpt(d,'Inline integer encoding contract',1,86)
for name in ['fork','pending_delta_len','compact']:item('crates/sparq-core/src/lib.rs',name,indent=4,public=True)
excerpt('crates/sparq-engine/Cargo.toml','engine features',25,42)
excerpt('crates/sparq-engine/Cargo.toml','core regular dependency',416,428)
excerpt('crates/sparq-engine/Cargo.toml','dev dependency feature unification',465,487)
excerpt('.github/workflows/ci.yml','workspace all-targets test archive caller',490,523)
(out/'source-context-index.json').write_text(json.dumps([{k:v for k,v in x.items() if k!='body'} for x in contexts],indent=2)+'\n')
parts=[]
for x in contexts:
 language='toml' if x['path'].endswith('.toml') else 'yaml' if x['path'].endswith('.yml') else 'rust'
 parts.append(f"### {x['path']}:{x['start']}-{x['end']} — {x['label']}\nBlob {x['source_blob']} at {head}.\n\n```{language}\n{x['body']}```\n")
(out/'source-context.md').write_text('\n'.join(parts))
print(json.dumps(dict(head=head,contexts=len(contexts),context_bytes=(out/'source-context.md').stat().st_size)))
