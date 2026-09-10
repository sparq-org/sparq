require 'yaml'
require 'json'
src=File.read('.github/workflows/js.yml')
def paths(text)
  doc=YAML.load(text)
  (doc[true] || doc['on'])['pull_request']['paths']
end
def covers(text, file)
  paths(text).any?{|glob| File.fnmatch(glob,file,File::FNM_PATHNAME)}
end
files=['site/package.json','gui/app/package.json']; raise unless files.all?{|f|covers(src,f)}
controls=files.map do |file|
  mutant=src.sub("      - \"#{file}\"\n",'')
  raise if mutant==src || covers(mutant,file)
  {removed:file,coverage_lost:true}
end
raise if covers(src,'unrelated/source.rs')
puts JSON.pretty_generate({actual_consumer_coverage:true,unrelated_path_stays_excluded:true,controls:controls})
