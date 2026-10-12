import {describe,expect,it} from 'vitest';
import {mkdtempSync,mkdirSync,writeFileSync,rmSync} from 'node:fs';
import {join} from 'node:path';
import {tmpdir} from 'node:os';
// @ts-expect-error Native build helper is a JavaScript entrypoint.
import {dllExports,validateComponent,componentSourceKey} from '../scripts/build-text-service.mjs';
function pe(names:string[]){
 const bytes=Buffer.alloc(4096);bytes.write('MZ');bytes.writeUInt32LE(0x80,0x3c);bytes.writeUInt32LE(0x4550,0x80);
 bytes.writeUInt16LE(0x8664,0x84);bytes.writeUInt16LE(1,0x86);bytes.writeUInt16LE(240,0x94);
 const optional=0x98;bytes.writeUInt16LE(0x20b,optional);bytes.writeUInt32LE(0x1000,optional+112);
 const section=optional+240;bytes.writeUInt32LE(0x1000,section+12);bytes.writeUInt32LE(0x800,section+16);bytes.writeUInt32LE(0x400,section+20);
 bytes.writeUInt32LE(names.length,0x400+24);bytes.writeUInt32LE(0x1040,0x400+32);
 let cursor=0x500;names.forEach((name,index)=>{bytes.writeUInt32LE(cursor-0x400+0x1000,0x440+index*4);bytes.write(name+'\0',cursor);cursor+=name.length+1;});return bytes;
}
describe('ordinary component build identities',()=>{
 it('reuses proxy across private core changes and invalidates full routing validation or toolchain changes',()=>{
  const root=mkdtempSync(join(tmpdir(),'sayweave-component-key-'));
  try{const base=join(root,'src-tauri/text-service');mkdirSync(join(base,'src'),{recursive:true});
   for(const file of ['Cargo.toml','Cargo.lock','build.rs','service.def','src/lib.rs','src/proxy.rs','src/proxy_transport.rs','src/engine_abi.rs','src/lifecycle.rs','src/routing.rs','src/transport.rs','src/registration.rs','src/host_identity.rs','src/deployment.rs','src/service.rs'])writeFileSync(join(base,file),'initial');
   const key=()=>componentSourceKey(root,'proxy',{},'rustc 1');const before=key();
   writeFileSync(join(base,'src/service.rs'),'changed text write');expect(key()).toBe(before);
   writeFileSync(join(base,'src/routing.rs'),'changed valid() and serde attributes');expect(key()).not.toBe(before);
   const routing=key();expect(componentSourceKey(root,'proxy',{RUSTFLAGS:'-C opt-level=2'},'rustc 1')).not.toBe(routing);
   expect(componentSourceKey(root,'proxy',{},'rustc 2')).not.toBe(routing);
  }finally{rmSync(root,{recursive:true,force:true});}
 });
 it('separates stable proxy from engine roles',()=>{const proxy=pe(['DllGetClassObject','DllCanUnloadNow']),core=pe(['DllGetClassObject','DllCanUnloadNow','CreateEngineV1']);
  expect(dllExports(core)).toContain('CreateEngineV1');expect(validateComponent(core,'core')).toMatch(/^[a-f0-9]{64}$/);
  expect(validateComponent(proxy,'proxy')).not.toBe(validateComponent(core,'core'));expect(()=>validateComponent(proxy,'core')).toThrow();expect(()=>validateComponent(core,'proxy')).toThrow();});
 it('refuses acceptance hooks in ordinary payloads',()=>{expect(()=>validateComponent(pe(['DllGetClassObject','DllCanUnloadNow','CreateEngineV1','SayweaveInitialFixtureFault']),'core')).toThrow();});
 it('rejects malformed offsets and wrong platform before loading',()=>{const malformed=pe(['DllGetClassObject','DllCanUnloadNow']);malformed.writeUInt32LE(0xffffffff,0x3c);expect(()=>dllExports(malformed)).toThrow();
  const wrong=pe(['DllGetClassObject','DllCanUnloadNow']);wrong.writeUInt16LE(0x14c,0x84);expect(()=>validateComponent(wrong,'proxy')).toThrow();});
});
