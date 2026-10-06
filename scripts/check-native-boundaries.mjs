import { readdirSync, readFileSync } from 'node:fs';
import path from 'node:path';

// Scoped ownership rules, not a claim to resolve all Rust call graphs or macros.
const root = path.resolve('src-tauri/src');
function files(dir, prefix = '') {
  return readdirSync(dir, { withFileTypes: true }).flatMap(entry => {
    const name = `${prefix}${entry.name}`;
    return entry.isDirectory() ? files(path.join(dir, entry.name), `${name}/`)
      : entry.name.endsWith('.rs') && !/(?:^|\/)(?:tests|.*_tests|.*acceptance|input_fixture)\.rs$/.test(name) ? [name] : [];
  });
}
function code(source) {
  return source.replace(/r(#+)"[\s\S]*?"\1/g, ' ').replace(/"(?:\\.|[^"\\])*"/g, ' ')
    .replace(/\/\*[\s\S]*?\*\//g, ' ').replace(/\/\/[^\n]*/g, ' ');
}
function roots(source) {
  const result = new Set([...source.matchAll(/\bcrate\s*::\s*(\w+)/g)].map(match => match[1]));
  for (const match of source.matchAll(/\buse\s+crate\s*::\s*\{/g)) {
    let depth = 0, cursor = match.index + match[0].length, fieldStart = true;
    for (; cursor < source.length; cursor++) {
      const c = source[cursor];
      if (c === '}' && depth === 0) break;
      if (c === '{') depth++;
      else if (c === '}') depth--;
      else if (c === ',' && depth === 0) fieldStart = true;
      else if (fieldStart && depth === 0 && /[A-Za-z_]/.test(c)) {
        const word = source.slice(cursor).match(/^\w+/)[0];
        result.add(word); cursor += word.length - 1; fieldStart = false;
      }
    }
  }
  return result;
}
const violations = [];
const checked = files(root);
for (const file of checked) {
  const source = code(readFileSync(path.join(root, file), 'utf8'));
  const refs = roots(source);
  const forbidden = file.startsWith('contracts/') ? [...refs].filter(name => name !== 'contracts')
    : file.startsWith('data/') ? [...refs].filter(name => name !== 'contracts' && name !== 'data')
    : file === 'network.rs' ? [...refs]
    : [];
  for (const name of forbidden) violations.push(`${file} -> crate::${name}`);
  if (/^(?:models|history|shortcuts)\//.test(file) && /\bimpl\s+Repository\b/.test(source)) {
    violations.push(`${file}: SQL Repository implementation belongs to data/repositories`);
  }
  if (file === 'speech/mod.rs' && /\bcrate\s*::\s*(?:Runtime|read|publish)\b|\bemit_to\s*\(/.test(source)) {
    violations.push(`${file}: application Runtime/event projection belongs to startup`);
  }
}
if (violations.length) throw new Error(`Native ownership rejected:\n${violations.join('\n')}`);
console.log(`Native ownership passed (${checked.length} sources; contracts/data/network and speech projection boundaries).`);
