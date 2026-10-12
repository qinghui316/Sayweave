import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';

const sha = bytes => createHash('sha256').update(bytes).digest('hex');
export function dllExports(bytes) {
  const within = (offset, size) => { if (!Number.isInteger(offset) || offset < 0 || offset + size > bytes.length) throw Error('Invalid component PE bounds'); return offset; };
  if (bytes.length < 256 || bytes.length > 32 * 1024 * 1024 || bytes.toString('ascii', 0, 2) !== 'MZ') throw Error('Invalid component PE');
  const pe = bytes.readUInt32LE(0x3c); within(pe, 24);
  if (bytes.readUInt32LE(pe) !== 0x4550 || bytes.readUInt16LE(pe + 4) !== 0x8664) throw Error('Component must be Windows x64');
  const optional = pe + 24, size = bytes.readUInt16LE(pe + 20), sections = bytes.readUInt16LE(pe + 6);
  within(optional, size + sections * 40);
  if (bytes.readUInt16LE(optional) !== 0x20b || size < 120 || sections > 96) throw Error('Invalid component headers');
  const address = rva => {
    for (let index = 0; index < sections; index++) {
      const at = optional + size + index * 40, virtual = bytes.readUInt32LE(at + 12), raw = bytes.readUInt32LE(at + 16);
      if (rva >= virtual && rva < virtual + raw) return within(bytes.readUInt32LE(at + 20) + rva - virtual, 1);
    }
    throw Error('Invalid component export address');
  };
  const exports = address(bytes.readUInt32LE(optional + 112)); within(exports, 40);
  const count = bytes.readUInt32LE(exports + 24); if (count > 4096) throw Error('Component export limit');
  const names = address(bytes.readUInt32LE(exports + 32)); within(names, count * 4);
  return Array.from({ length: count }, (_, index) => {
    const start = address(bytes.readUInt32LE(names + index * 4)), end = bytes.indexOf(0, start);
    if (end < start || end - start > 256) throw Error('Invalid component export name');
    return bytes.toString('ascii', start, end);
  });
}
export function validateComponent(bytes, role) {
  const exports = dllExports(bytes);
  if (!exports.includes('DllGetClassObject') || !exports.includes('DllCanUnloadNow')
    || (role === 'core') !== exports.includes('CreateEngineV1')
    || exports.some(name => /Fixture|Fault/.test(name))) throw Error(`Invalid ordinary ${role} exports`);
  return sha(bytes);
}
function run(root, args, env) {
  const result = spawnSync('cargo', args, { cwd: root, env, stdio: 'inherit', windowsHide: true });
  if (result.error) throw result.error;
  if (result.status !== 0) throw Error(`Component build failed (${result.status})`);
}
function sources(directory) {
  return readdirSync(directory, { withFileTypes: true }).flatMap(entry => entry.isDirectory()
    ? sources(path.join(directory, entry.name)) : [path.join(directory, entry.name)]).sort();
}
export function componentSourceKey(root, role, env, toolchain) {
  const base = path.join(root, 'src-tauri/text-service');
  const files = role === 'core' ? sources(path.join(base, 'src')).filter(file => !['probe.rs','core_switch_probe.rs','proxy.rs','proxy_transport.rs'].includes(path.basename(file))) : [
    'src/lib.rs', 'src/proxy.rs', 'src/proxy_transport.rs', 'src/engine_abi.rs', 'src/lifecycle.rs',
    'src/routing.rs', 'src/transport.rs', 'src/registration.rs', 'src/host_identity.rs', 'src/deployment.rs',
  ].map(file => path.join(base, file));
  const rawManifest = readFileSync(path.join(base, 'Cargo.toml'), 'utf8');
  const manifest = role === 'proxy' ? rawManifest.replace(/^version\s*=.*$/m, 'version = "proxy-independent"') : rawManifest;
  const inputs = { role, abi: 1, profile: 'release', platform: process.platform, architecture: process.arch, toolchain,
    flags: Object.fromEntries(Object.entries(env).filter(([name]) => /^(?:RUSTFLAGS|CARGO_ENCODED_RUSTFLAGS|RUSTC|RUSTC_WRAPPER|RUSTC_WORKSPACE_WRAPPER|CARGO_BUILD_|CARGO_PROFILE_)/.test(name)).sort()),
    manifest, lock: sha(readFileSync(path.join(base, 'Cargo.lock'))), build: sha(readFileSync(path.join(base, 'build.rs'))),
    exports: sha(readFileSync(path.join(base, 'service.def'))),
    sources: files.map(file => [path.relative(base, file), sha(readFileSync(file))]),
    ...(role === 'core' ? { buildId: env.SAYWEAVE_ENGINE_BUILD_ID ?? '0' } : {}),
  };
  return sha(JSON.stringify(inputs));
}
export function buildTextService(root, env = process.env) {
  if (process.platform !== 'win32' || process.arch !== 'x64') throw Error('TSF currently requires Windows x64');
  const compiler = spawnSync('rustc', ['-vV'], { cwd: root, env, encoding: 'utf8', windowsHide: true });
  if (compiler.status !== 0) throw Error('Cannot verify component compiler');
  const cache = path.join(root, '.local/text-service-build'); mkdirSync(cache, { recursive: true });
  const output = path.join(root, 'src-tauri/text-service/target/release/sayweave_text_service.dll');
  function build(role, supplied) {
    const source = componentSourceKey(root, role, env, compiler.stdout), recordFile = path.join(cache, `${role}-${source}.json`);
    if (existsSync(recordFile)) {
      const record = JSON.parse(readFileSync(recordFile, 'utf8'));
      const bytes = existsSync(record.path) ? readFileSync(record.path) : null;
      if (record.source !== source || (bytes && validateComponent(bytes, role) !== record.sha256)) throw Error('Component cache identity differs');
      if (bytes && ((!record.supplied && !supplied) || (supplied && sha(readFileSync(supplied)) === record.sha256))) { console.log(`Reusing verified ${role} ${record.sha256}`); return record.path; }
    }
    if (!supplied) run(root, ['build', '--locked', '--manifest-path', 'src-tauri/text-service/Cargo.toml', '--release', '--lib',
      ...(role === 'proxy' ? ['--no-default-features', '--features', 'proxy'] : ['--features', 'engine'])],
      { ...env, ...(role === 'proxy' ? { SAYWEAVE_ENGINE_BUILD_ID: '0' } : {}) });
    const bytes = readFileSync(supplied ?? output), digest = validateComponent(bytes, role), directory = path.join(cache, digest);
    mkdirSync(directory, { recursive: true }); const immutable = path.join(directory, `${role}.dll`);
    if (existsSync(immutable)) { if (!readFileSync(immutable).equals(bytes)) throw Error('Immutable component bytes differ'); }
    else writeFileSync(immutable, bytes, { flag: 'wx' });
    // Preserve the producer identity when artifact bytes are reused later.
    writeFileSync(recordFile, JSON.stringify({ schema: 1, role, source, sha256: digest, path: immutable, compiler: compiler.stdout,
      sourceCommit: supplied ? null : spawnSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8', windowsHide: true }).stdout.trim(), supplied: Boolean(supplied), sourceTreeDirty: supplied ? null : Boolean(spawnSync('git', ['status','--porcelain','--untracked-files=normal'], {cwd:root,encoding:'utf8',windowsHide:true}).stdout.trim()) }, null, 2));
    return immutable;
  }
  // Snapshot a supplied core before a proxy build replaces Cargo's public output filename.
  const core = build('core', env.SAYWEAVE_TSF_PAYLOAD), proxy = build('proxy', env.SAYWEAVE_TSF_PROXY_PAYLOAD);
  return { core, proxy };
}
