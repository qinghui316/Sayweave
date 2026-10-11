import {describe,expect,it} from 'vitest';
// @ts-expect-error Checked native JavaScript entrypoint.
import {assertFrozenFile} from '../scripts/frozen-product-scope.mjs';
describe('frozen product source boundaries',()=>{
 const cargo='[package]\nname="x"\n[dependencies]\nwindows="1"\n';
 it('allows only the approved dev/test profile additions',()=>{expect(assertFrozenFile('src-tauri/Cargo.toml',cargo,cargo+'[profile.dev]\ndebug=1\nincremental=true\n[profile.test]\ndebug=1\nincremental=true\n')).toBe('development-profile-only');});
 it.each(['[profile.release]\ndebug=1','[profile.dev]\ndebug=2\nincremental=false','[profile.dev]\ndebug=1\nincremental=true\nopt-level=3'])('rejects product or unrelated profile changes',extra=>{expect(()=>assertFrozenFile('src-tauri/Cargo.toml',cargo,cargo+extra)).toThrow();});
 it('rejects dependency changes and business source modifications',()=>{expect(()=>assertFrozenFile('src-tauri/Cargo.toml',cargo,cargo.replace('windows="1"','windows="2"'))).toThrow();expect(()=>assertFrozenFile('src-tauri/src/lib.rs','','')).toThrow();});
 it('accepts only the exact storage commands without changing product metadata',()=>{const before={version:'0.1.9',dependencies:{a:'1'},scripts:{build:'vite build','test:native':'cargo test --manifest-path src-tauri/Cargo.toml'}},after=structuredClone(before);after.scripts['test:native']='node scripts/storage-cargo.mjs test --manifest-path src-tauri/Cargo.toml';expect(assertFrozenFile('package.json',JSON.stringify(before),JSON.stringify(after))).toBe('storage-commands-only');after.version='0.2.0';expect(()=>assertFrozenFile('package.json',JSON.stringify(before),JSON.stringify(after))).toThrow();});
});
