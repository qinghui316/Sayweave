import {describe,expect,it} from 'vitest';
import {mkdtempSync,mkdirSync,writeFileSync,readFileSync,rmSync} from 'node:fs';
import {join} from 'node:path';
import {tmpdir} from 'node:os';
import {createHash} from 'node:crypto';
// @ts-expect-error Native build helper is a JavaScript entrypoint.
import {dllExports,validateComponent,componentSourceKey,pinnedProxy,proxySourceIdentity,proxySourceFiles,verifyComponentEmbeddings} from '../scripts/build-text-service.mjs';
// @ts-expect-error Native gate helper is a JavaScript entrypoint.
import {extractBaseline} from '../scripts/test-formal-core-switch.mjs';
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
  try{const base=join(root,'src-tauri/text-service');mkdirSync(join(base,'src/deployment'),{recursive:true});
   for(const file of [...proxySourceFiles,'src/service.rs'])writeFileSync(join(base,file),'initial');
   const key=()=>componentSourceKey(root,'proxy',{},'rustc 1');const before=key();
   writeFileSync(join(base,'src/service.rs'),'changed text write');expect(key()).toBe(before);
   writeFileSync(join(base,'src/routing.rs'),'changed valid() and serde attributes');expect(key()).not.toBe(before);
   const deployment=key();writeFileSync(join(base,'src/deployment/retired.rs'),'changed cleanup');expect(key()).not.toBe(deployment);
   const retirement=key();writeFileSync(join(base,'src/atomic_record.rs'),'changed retry');expect(key()).not.toBe(retirement);
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
describe('pinned formal proxy',()=>{
 function fixture(){
  const root=mkdtempSync(join(tmpdir(),'sayweave-pinned-'));
  const base=join(root,'src-tauri/text-service'),resource=join(root,'src-tauri/resources/fixed-proxy');
  mkdirSync(join(base,'src/deployment'),{recursive:true});mkdirSync(resource,{recursive:true});
  for(const file of proxySourceFiles)writeFileSync(join(base,file),'initial');
  writeFileSync(join(root,'rust-toolchain.toml'),'pinned rust');
  const bytes=pe(['DllGetClassObject','DllCanUnloadNow']);writeFileSync(join(resource,'abi1.dll'),bytes);
  const manifest={schemaVersion:1,abi:1,platform:'windows-x86_64-msvc',file:'abi1.dll',size:bytes.length,sha256:validateComponent(bytes,'proxy'),source:proxySourceIdentity(root),origin:{tag:'v0.1.12',commit:'a'.repeat(40),repository:'qinghui316/Sayweave',installer:'Sayweave_0.1.12_x64-setup.exe',installerSha256:'b'.repeat(64),mainSha256:'c'.repeat(64),helperSha256:'d'.repeat(64)}};
  const save=()=>writeFileSync(join(resource,'manifest.json'),JSON.stringify(manifest));save();
  return {root,base,resource,bytes,manifest,save};
 }
 it('uses identical ordinary bytes with no cache and rejects arbitrary supplied proxies',()=>{
  const f=fixture();try{expect(pinnedProxy(f.root).manifest.sha256).toBe(f.manifest.sha256);expect(pinnedProxy(f.root).path).toBe(pinnedProxy(f.root).path);
   const other=join(f.root,'other.dll');writeFileSync(other,Buffer.from('wrong'));expect(()=>pinnedProxy(f.root,other)).toThrow('Supplied proxy');
   writeFileSync(join(f.resource,'abi1.dll'),Buffer.from('wrong'));expect(()=>pinnedProxy(f.root)).toThrow();
  }finally{rmSync(f.root,{recursive:true,force:true});}
 });
 it.each(['src/wire.rs','src/client.rs','src/settlement_boundary.rs','src/lib.rs','src/deployment/retired.rs','Cargo.lock','build.rs'])('fails closed on changed proxy dependency %s',file=>{
  const f=fixture();try{writeFileSync(join(f.base,file),'changed');expect(()=>pinnedProxy(f.root)).toThrow('source contract');}finally{rmSync(f.root,{recursive:true,force:true});}
 });
 it('ignores private core changes but rejects wrong ABI/platform/origin and missing bytes',()=>{
  const f=fixture();try{writeFileSync(join(f.base,'src/engine.rs'),'new core');expect(pinnedProxy(f.root).manifest.sha256).toBe(f.manifest.sha256);
   f.manifest.abi=2;f.save();expect(()=>pinnedProxy(f.root)).toThrow();f.manifest.abi=1;
   f.manifest.platform='linux';f.save();expect(()=>pinnedProxy(f.root)).toThrow();f.manifest.platform='windows-x86_64-msvc';
   f.manifest.origin.repository='other/repo';f.save();expect(()=>pinnedProxy(f.root)).toThrow();f.manifest.origin.repository='qinghui316/Sayweave';f.save();
   rmSync(join(f.resource,'abi1.dll'));expect(()=>pinnedProxy(f.root)).toThrow();
  }finally{rmSync(f.root,{recursive:true,force:true});}
 });
 it('binds executable payloads to the same proxy and core',()=>{
  const f=fixture();try{const core=join(f.root,'core.dll'),main=join(f.root,'main.exe'),helper=join(f.root,'helper.exe');writeFileSync(core,pe(['DllGetClassObject','DllCanUnloadNow','CreateEngineV1']));
   const payload=Buffer.concat([Buffer.from('MZ'),f.bytes,readFileSync(core)]);writeFileSync(main,payload);writeFileSync(helper,payload);
   expect(verifyComponentEmbeddings([main,helper],{proxy:pinnedProxy(f.root).path,core}).proxySha256).toBe(f.manifest.sha256);
   writeFileSync(helper,Buffer.concat([Buffer.from('MZ'),readFileSync(core)]));expect(()=>verifyComponentEmbeddings([main,helper],{proxy:pinnedProxy(f.root).path,core})).toThrow('payload differs');
  }finally{rmSync(f.root,{recursive:true,force:true});}
 });
 it('extracts only exact components from both authenticated baseline EXEs',()=>{
  const f=fixture();try{
   const core=pe(['DllGetClassObject','DllCanUnloadNow','CreateEngineV1']);
   const executable=Buffer.concat([Buffer.from('MZ'),f.bytes,core]);const digest=createHash('sha256').update(executable).digest('hex');
   const manifest={...f.manifest,origin:{...f.manifest.origin,mainSha256:digest,helperSha256:digest,mainOffset:2,helperOffset:2,core:{sha256:validateComponent(core,'core'),size:core.length,mainOffset:2+f.bytes.length,helperOffset:2+f.bytes.length}}};
   expect(extractBaseline(executable,executable,manifest).proxy).toEqual(f.bytes);
   expect(extractBaseline(executable,executable,manifest).core).toEqual(core);
   expect(()=>extractBaseline(executable,Buffer.from('wrong'),manifest)).toThrow('executable identity');
   manifest.origin.core.mainOffset++;expect(()=>extractBaseline(executable,executable,manifest)).toThrow();
  }finally{rmSync(f.root,{recursive:true,force:true});}
 });
});
