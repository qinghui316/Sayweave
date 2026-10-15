import { execFileSync } from 'node:child_process';
import { readdirSync } from 'node:fs';
import path from 'node:path';

// Keep the source distribution explicit. Add new public files here during review.
const roots = new Set(['.gitattributes', '.gitignore', 'README.md', 'LICENSE', 'NOTICE',
  'CONTRIBUTING.md', 'SECURITY.md', 'CHANGELOG.md', 'package.json', 'package-lock.json',
  'eslint.config.js', 'index.html', 'playwright.config.ts', 'rust-toolchain.toml',
  'tsconfig.json', 'vite.config.ts']);
const docs = new Set(['architecture.md', 'revisable-draft.md', 'development.md', 'design.md', 'testing.md', 'local-data.md', 'motion.md', 'models.md', 'speech.md', 'history.md', 'dictation.md', 'personalization.md', 'release.md','storage.md']);
const readmeMedia = new Set([
  'brand-light.svg', 'brand-dark.svg',
  'hero.gif', 'comparison.gif', 'cleanup.gif', 'handoff.gif', 'dictionary.gif', 'personalization.gif',
  'home.webp', 'words.webp', 'history.webp', 'appearance.webp', 'provenance.md', 'manifest.json',
].map(file => `images/readme/${file}`));
const releaseNotes = /^release-notes\/v(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.json$/;
const publicWorkflow = file => ['windows-release.yml','windows-ci.yml'].includes(file.replace('.github/workflows/','')) && file.startsWith('.github/workflows/');
const scripts = new Set(["windows-update-debug-policy.mjs",'release-validation.mjs','test-windows-installed-startup.mjs','windows-update-startup.mjs','windows-update-packages.mjs','windows-update-registration.mjs','windows-update-artifacts.mjs','reuse-windows-update-evidence.mjs','windows-update-fixtures.mjs','test-windows-update.mjs','test-windows-update.ps1','check-boundaries.mjs', 'check-native-boundaries.mjs', 'check-text.mjs', 'check-distribution.mjs', 'generate-update-manifest.mjs', 'verify-update-release.mjs',
  'test-desktop.ps1', 'test-dictionary-desktop.mjs', 'test-models-desktop.mjs', 'test-speech-desktop.mjs', 'test-prefix-input.mjs', 'test-richtext-block.mjs', 'test-input.ps1', 'test-core-switch.ps1', 'build-text-service.mjs', 'desktop.mjs', 'test-language-actor.mjs', 'test-language-dashboard-live.mjs']);
const testRoots = new Set(['contracts', 'e2e']);
scripts.add('test-formal-core-switch.mjs');
scripts.add('windows-update-metadata.cs');
scripts.add('windows-update-metadata.ps1');
for(const script of ['storage.mjs','storage-policy.mjs','storage-safety.ps1','storage-cargo.mjs'])scripts.add(script);
const testFixtures = new Set(['voice-recovery.html', 'voice-recovery.tsx', 'input-target.ts', 'faithful-v1.txt']);
const generated = /(?:^|\/)(?:node_modules|target|gen|dist|coverage|test-results|playwright-report|\.local|\.git)(?:\/|$)|^tests\/visual\//i;
const ownerLocal = /(?:^|\/)(?:\.agents|\.claude|\.codex|\.impeccable)(?:\/|$)|(?:^|\/)(?:AGENTS\.md|CLAUDE\.md|\.env[^/]*|dev\.ps1|doctor\.mjs|environment\.ps1|setup-tools\.ps1|harness-skill-link\.(?:mjs|ps1|py))$|\.cmd$/i;
function allowed(file) {
  if (generated.test(file) || ownerLocal.test(file) || /\.(?:sqlite(?:-journal|-shm|-wal)?|db|log|fig|wav|mp3)$/i.test(file)) return false;
  if (roots.has(file)) return true;
  if (publicWorkflow(file)) return true;
  if (file.startsWith('src/') || file.startsWith('src-tauri/')) return true;
  if (file.startsWith('docs/')) return docs.has(file.slice(5)) || readmeMedia.has(file.slice(5));
  if (releaseNotes.test(file)) return true;
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
