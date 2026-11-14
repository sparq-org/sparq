// Research grade and not externally audited.
import { readFileSync, readdirSync, existsSync, mkdirSync, writeFileSync, realpathSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { homedir, tmpdir } from 'node:os';
import { dirname, join, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
const root = fileURLToPath(new URL('../../../', import.meta.url));
const assets = fileURLToPath(new URL('../assets/', import.meta.url));
const sha = bytes => createHash('sha256').update(bytes).digest('hex');
const slash = path => path.split(sep).join('/');

export function snapshotSources() {
  const files = {}, seen = new Map(), origins = [{ source: realpathSync(root), target: '' }], revisions = {};
  function targetFor(source) {
    const origin = origins.filter(p => source === p.source || source.startsWith(p.source + sep)).sort((a,b) => b.source.length - a.source.length)[0];
    if (!origin) throw new Error('source dependency is outside a pinned library');
    return slash(join(origin.target, relative(origin.source, source)));
  }
  function gitSource(url, tag, directory = '') {
    const match = /^https:\/\/github\.com\/([\w-]+)\/([\w.-]+?)(?:\.git)?$/.exec(url);
    if (!match || !/^v[\w.-]+$/.test(tag)) throw new Error('Noir dependency needs a tagged source');
    const [,owner,repository] = match;
    let source = join(homedir(), 'nargo', 'github.com', owner, repository, tag);
    if (!existsSync(source)) {
      source = join(tmpdir(), 'sparq-query-source-cache', owner, repository, tag);
      if (!existsSync(source)) {
        mkdirSync(dirname(source), { recursive: true });
        execFileSync('git', ['clone', '--quiet', '--depth', '1', '--branch', tag, url, source], { stdio: 'inherit' });
      }
    }
    source = realpathSync(source);
    execFileSync('git', ['-C', source, 'diff', '--quiet', 'HEAD', '--']);
    const revision = execFileSync('git', ['-C', source, 'rev-parse', 'HEAD'], { encoding: 'utf8' }).trim();
    const target = slash(join('noir-deps', owner, repository, tag));
    if (!origins.some(o => o.source === source)) origins.push({ source, target });
    revisions[`${url}@${tag}`] = revision;
    for (const name of ['LICENSE', 'LICENSE-MIT', 'LICENSE-APACHE', 'COPYING']) {
      if (existsSync(join(source, name))) files[`${target}/${name}`] = readFileSync(join(source, name), 'utf8');
    }
    return resolve(source, directory);
  }
  function visit(source) {
    source = realpathSync(source);
    if (seen.has(source)) return seen.get(source);
    const target = targetFor(source); seen.set(source, target);
    for (const name of ['LICENSE', 'LICENSE-MIT', 'LICENSE-APACHE', 'COPYING', 'VENDOR-PROVENANCE.md']) {
      if (existsSync(join(source, name))) files[`${target}/${name}`] = readFileSync(join(source, name), 'utf8');
    }
    const original = readFileSync(join(source, 'Nargo.toml'), 'utf8');
    const lines = original.split('\n').map(line => {
      const match = /^(\s*[\w-]+\s*=\s*)\{([^}]+)\}(.*)$/.exec(line);
      if (!match) return line;
      const fields = Object.fromEntries([...match[2].matchAll(/([\w_]+)\s*=\s*"([^"]*)"/g)].map(m => [m[1],m[2]]));
      let dependency;
      if (fields.path) dependency = resolve(source, fields.path);
      else if (fields.git && fields.tag) dependency = gitSource(fields.git, fields.tag, fields.directory);
      else throw new Error('Noir dependency manifest rejected');
      const destination = visit(dependency);
      const local = slash(relative(target, destination)) || '.';
      return `${match[1]}{ path = ${JSON.stringify(local)} }${match[3]}`;
    });
    files[`${target}/Nargo.toml`] = lines.join('\n');
    function copy(path) {
      for (const entry of readdirSync(path, { withFileTypes: true })) {
        const full = join(path, entry.name);
        if (entry.isDirectory() && !['target','.git','.github'].includes(entry.name)) copy(full);
        else if (entry.isFile() && entry.name.endsWith('.nr')) files[`${target}/${slash(relative(source, full))}`] = readFileSync(full, 'utf8');
      }
    }
    copy(source);
    return target;
  }
  visit(join(root, 'zk/noir-query/core'));
  const sorted = Object.fromEntries(Object.keys(files).sort().map(name => [name, files[name]]));
  const bundle = Buffer.from(JSON.stringify({ files: sorted, revisions }) + '\n');
  writeFileSync(join(assets, 'sources.json'), bundle);
  return sha(bundle);
}
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const digest = snapshotSources();
  const tablePath = join(assets, 'catalogue.json'), table = JSON.parse(readFileSync(tablePath));
  if (process.argv.includes('--write-pins')) {
    table.sourceBundleSha256 = digest;
    writeFileSync(tablePath, JSON.stringify(table, null, 2) + '\n');
  } else if (table.sourceBundleSha256 !== digest) throw new Error('source bundle differs from catalogue pin');
}
