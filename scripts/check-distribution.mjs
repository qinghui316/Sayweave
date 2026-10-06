import { execFileSync } from 'node:child_process';
import { readdirSync } from 'node:fs';
import path from 'node:path';

// Keep the source distribution explicit. Add new public files here during review.
const roots = new Set(['.gitattributes', '.gitignore', 'README.md', 'LICENSE',
  'CONTRIBUTING.md', 'SECURITY.md', 'CHANGELOG.md', 'package.json', 'package-lock.json',
  'eslint.config.js', 'index.html', 'playwright.config.ts', 'rust-toolchain.toml',
  'tsconfig.json', 'vite.config.ts']);
const docs = new Set(['architecture.md', 'development.md', 'design.md', 'testing.md', 'local-data.md', 'motion.md', 'models.md', 'speech.md', 'history.md', 'dictation.md']);
const scripts = new Set(['check-boundaries.mjs', 'check-native-boundaries.mjs', 'check-text.mjs', 'check-distribution.mjs',
  'test-desktop.ps1', 'test-dictionary-desktop.mjs', 'test-models-desktop.mjs', 'test-speech-desktop.mjs', 'test-input.ps1', 'desktop.mjs']);
const testRoots = new Set(['contracts', 'e2e']);
const testFixtures = new Set(['voice-recovery.html', 'voice-recovery.tsx', 'input-target.ts']);
const generated = /(?:^|\/)(?:node_modules|target|gen|dist|coverage|test-results|playwright-report|\.local|\.git)(?:\/|$)|^tests\/visual\//i;
const ownerLocal = /(?:^|\/)(?:\.agents|\.claude|\.codex|\.impeccable)(?:\/|$)|(?:^|\/)(?:AGENTS\.md|CLAUDE\.md|\.env[^/]*|dev\.ps1|doctor\.mjs|environment\.ps1|setup-tools\.ps1|harness-skill-link\.(?:mjs|ps1|py))$|\.cmd$/i;
function allowed(file) {
  if (generated.test(file) || ownerLocal.test(file) || /\.(?:sqlite(?:-journal|-shm|-wal)?|db|log|fig|wav|mp3)$/i.test(file)) return false;
  if (roots.has(file)) return true;
  if (file.startsWith('src/') || file.startsWith('src-tauri/')) return true;
  if (file.startsWith('docs/')) return docs.has(file.slice(5));
  if (file.startsWith('scripts/')) return scripts.has(file.slice(8));
  if (file.startsWith('tests/')) {
    const remainder = file.slice(6);
    return testRoots.has(remainder.split('/')[0]) || remainder.startsWith('fixtures/') && testFixtures.has(remainder.slice(9)) || /^[^/]+\.(?:test\.tsx?|ts)$/.test(remainder);
  }
  return false;
}
function snapshotFiles(root, prefix = '') {
  return readdirSync(path.join(root, prefix), { withFileTypes: true }).flatMap(entry => {
    const file = `${prefix}${entry.name}`;
    if (generated.test(file)) return [];
    if (entry.isSymbolicLink()) throw new Error(`Source snapshot contains a link: ${file}`);
    return entry.isDirectory() ? snapshotFiles(root, `${file}/`) : [file];
  });
}
const args = process.argv.slice(2);
let files;
if (args[0] === '--snapshot' && args.length === 2) files = snapshotFiles(path.resolve(args[1]));
else if (args[0] === '--tree' && args.length === 2) files = execFileSync('git', ['ls-tree', '-r', '--name-only', '-z', args[1]], { encoding: 'utf8' }).split('\0').filter(Boolean);
else if (args.length === 0) files = execFileSync('git', ['ls-files', '-z'], { encoding: 'utf8' }).split('\0').filter(Boolean);
else throw new Error('Usage: node scripts/check-distribution.mjs [--tree REV | --snapshot DIRECTORY]');
if (files.length === 0) throw new Error('No source files found. Extracted archives must use --snapshot DIRECTORY.');
const rejected = files.filter(file => !allowed(file));
if (rejected.length) {
  console.error(`Source distribution rejected ${rejected.length} file(s):\n${rejected.join('\n')}`);
  process.exitCode = 1;
} else console.log(`Source distribution passed (${files.length} files).`);
