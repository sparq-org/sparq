// Research grade and not externally audited. Rendering only; Rust owns the layout.
import { readFileSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
const directory = new URL('../', import.meta.url);
const host = createRequire(import.meta.url)('../wasm/sparq_zk_query_wasm.cjs');
export function layoutSection() {
  const catalogue = JSON.parse(readFileSync(new URL('assets/catalogue.json', directory)));
  const entry = catalogue.entries[0];
  const sample = JSON.parse(readFileSync(new URL('../test/fixtures/authenticated.json', import.meta.url))).request;
  const artifact = JSON.parse(readFileSync(new URL('../test/fixtures/layout-abi.json', import.meta.url)));
  const info = JSON.parse(host.hostCall('layout', JSON.stringify({ query: entry.example.query, policy: { ...sample.policy, bounds: entry.example.bounds }, artifact })));
  const select = JSON.parse(readFileSync(new URL('../test/fixtures/layout-select-abi.json', import.meta.url)));
  host.hostCall('layout', JSON.stringify({ query: select.query, policy: { ...sample.policy, bounds: select.bounds }, artifact: { abi: select.abi } }));
  const declarations = info.declarations.trim().split('\n').map(line => line.trim().replace(/; \d+\]/g, '; length]').replace(/,$/, ''));
  const cell = artifact.abi.parameters.find(p => p.name === 'parameters').type.type;
  const fields = cell.fields.map(field => `\`${field.name}\``).join(', ');
  const tableFields = select.abi.parameters.find(p => p.name === 'result').type.fields.map(f => '`' + f.name + '`').join(', ');
  return '\n\n' + declarations.map(line => '- `' + line.replace(/: pub /, ': ') + '`').join('\n') + '\n\nCell field order: ' + fields + '.\nSELECT table field order: ' + tableFields + '.\n';
}
export function generatedReadme() {
  const path = new URL('README.md', directory), text = readFileSync(path, 'utf8');
  for (const marker of ['public-input-layout:start', 'public-input-layout:end', 'catalogue-pending:start', 'catalogue-pending:end']) {
    if (text.split('<!-- ' + marker + ' -->').length !== 2) throw new Error('generated README section missing or repeated');
  }
  const table = JSON.parse(readFileSync(new URL('assets/catalogue.json', directory)));
  const names = (table.notYetCatalogued || []).map(entry => '`' + entry.case + '`').join(', ');
  const failures = (table.nativeKnownFailures || []).map(f => '`' + f.case + '`').join(', ');
  const pending = (failures ? '\n\nNamed native witness known failures: ' + failures + '. They are exercised as\nrefusals and are not counted as proved; the catalogue records their cause.' : '') + (names ? '\n\nNot yet catalogued on this branch: ' + names + '. Native maintainer compilation\nis still running. These plans receive the named missing-catalogue refusal.\n' : '\n\nAll listed test profiles have pinned catalogue entries.\n');
  return text.replace(/(<!-- public-input-layout:start -->)[\s\S]*?(<!-- public-input-layout:end -->)/, '$1' + layoutSection() + '$2')
    .replace(/(<!-- catalogue-pending:start -->)[\s\S]*?(<!-- catalogue-pending:end -->)/, '$1' + pending + '$2');
}
if (process.argv[1] === fileURLToPath(import.meta.url)) writeFileSync(new URL('README.md', directory), generatedReadme());
