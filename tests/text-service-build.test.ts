import {describe,expect,it} from 'vitest';
import {mkdtempSync,mkdirSync,writeFileSync,readFileSync,rmSync} from 'node:fs';
import {join} from 'node:path';
import {tmpdir} from 'node:os';
import {createHash} from 'node:crypto';
// @ts-expect-error Native build helper is a JavaScript entrypoint.
import {dllExports,validateComponent,componentSourceKey,pinnedProxy,selectedProxy,proxySourceIdentity,proxySourceFiles,verifyComponentEmbeddings,ordinaryProxyCommand} from '../scripts/build-text-service.mjs';
// @ts-expect-error Native gate helper is a JavaScript entrypoint.
import {extractBaseline,validateFormalEvidence} from '../scripts/test-formal-core-switch.mjs';
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

describe('formal proxy upgrade receipt',()=>{
 const oldProxy='a'.repeat(64),newProxy='b'.repeat(64),oldCore='c'.repeat(64),newCore='d'.repeat(64);
 const manifest={schemaVersion:2,sha256:newProxy,baseline:{tag:'v0.1.20',commit:'a'.repeat(40),repository:'qinghui316/Sayweave',installer:'Sayweave_0.1.20_x64-setup.exe',installerSha256:'b'.repeat(64),mainSha256:'c'.repeat(64),helperSha256:'d'.repeat(64),proxy:{sha256:oldProxy},core:{sha256:oldCore}}};
 const record=(pid:number,proxy:string,core:string,generation:number,body:string)=>({body,runtime:{process:pid,proxy:{sha256:proxy,base:100},core:{sha256:core},engine:{proxy_instance:'instance-'+pid,engine_generation:generation}}});
 const target=(pid:number,phase:string,records:ReturnType<typeof record>[])=>({formalTargetPassed:true,cleanupComplete:true,moduleUnloadReady:phase!=='old',pid,phase,processIdentity:{pid},sameService:true,ordinaryTransport:true,records,finalBody:records.at(-1)!.body});
 const evidence=()=>({formalProxySwitchPassed:true,proxyUpgradePassed:true,oldTargetExited:true,newTargetExited:true,coreChanged:true,oldTarget:{...target(1,'old',[record(1,oldProxy,oldCore,1,'旧核心输入。')]),oldAdmissionRejected:'enginePrepare:80070005'},newTarget:target(2,'new',[record(2,newProxy,oldCore,1,'旧核心输入。'),record(2,newProxy,newCore,2,'旧核心输入。新核心输入。')])});
 it('requires actual old/new process and loaded component/body identities',()=>{
  const value=evidence();expect(()=>validateFormalEvidence(value,manifest,newCore)).not.toThrow();
  value.newTarget.records[1]!.runtime.proxy.sha256=oldProxy;expect(()=>validateFormalEvidence(value,manifest,newCore)).toThrow('Loaded');
 });
 it.each(['body','exit','generation','instance','reopen','refusal','unload','cleanup'])('refuses incomplete %s proof',kind=>{
  const v=evidence();
  if(kind==='body')v.newTarget.finalBody='';if(kind==='exit')v.oldTargetExited=false;if(kind==='generation')v.newTarget.records[1]!.runtime.engine.engine_generation=1;if(kind==='instance')v.newTarget.records[1]!.runtime.engine.proxy_instance='different';if(kind==='reopen')v.newTarget.pid=1;if(kind==='refusal')v.oldTarget.oldAdmissionRejected='engineBusy';if(kind==='unload')v.newTarget.moduleUnloadReady=false;if(kind==='cleanup')v.newTarget.cleanupComplete=false;
  expect(()=>validateFormalEvidence(v,manifest,newCore)).toThrow();
 });
 it('records unchanged-core input without pretending a generation switch occurred',()=>{
  const v=evidence();v.coreChanged=false;v.newTarget.records.pop();v.newTarget.finalBody='旧核心输入。';
  expect(()=>validateFormalEvidence(v,manifest,oldCore)).not.toThrow();
 });
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
 it('binds a source-built v2 proxy separately from its signed old-release baseline',()=>{
  const f=fixture();try{
   const digest=(value:Buffer|string)=>createHash('sha256').update(value).digest('hex');
   const old=f.bytes,core=pe(['DllGetClassObject','DllCanUnloadNow','CreateEngineV1']);
   const executable=Buffer.concat([Buffer.from('MZ'),old,core]);
   const bytes=pe(['DllGetClassObject','DllCanUnloadNow','NewOrdinaryBuild']);
   const compiler='rustc fixture\nhost: x86_64-pc-windows-msvc\n';
   const manifest={...f.manifest,schemaVersion:2,sha256:validateComponent(bytes,'proxy'),size:bytes.length,
    origin:{kind:'reviewed-source-build',sourceState:'recorded-source-snapshot',producerHead:'a'.repeat(40),sourceDigest:digest(JSON.stringify(f.manifest.source)),compiler,compilerSha256:digest(compiler),command:ordinaryProxyCommand,flags:{}},
    baseline:{...f.manifest.origin,tag:'v0.1.20',installer:'Sayweave_0.1.20_x64-setup.exe',mainSha256:digest(executable),helperSha256:digest(executable),proxy:{sha256:validateComponent(old,'proxy'),size:old.length,mainOffset:2,helperOffset:2},core:{sha256:validateComponent(core,'core'),size:core.length,mainOffset:2+old.length,helperOffset:2+old.length}}};
   const save=()=>writeFileSync(join(f.resource,'manifest.json'),JSON.stringify(manifest));
   writeFileSync(join(f.resource,'abi1.dll'),bytes);save();
   expect(pinnedProxy(f.root).manifest.sha256).not.toBe(manifest.baseline.proxy.sha256);
   expect(extractBaseline(executable,executable,manifest).proxy).toEqual(old);
   manifest.origin.compilerSha256='e'.repeat(64);save();expect(()=>pinnedProxy(f.root)).toThrow('provenance');
   manifest.origin.compilerSha256=digest(compiler);manifest.origin.command=[...ordinaryProxyCommand,'--features','acceptance'];save();expect(()=>pinnedProxy(f.root)).toThrow('provenance');
   manifest.origin.command=ordinaryProxyCommand;manifest.baseline.proxy.sha256='e'.repeat(64);save();expect(()=>extractBaseline(executable,executable,manifest)).toThrow('embedded component');
   manifest.baseline.proxy.sha256=validateComponent(old,'proxy');manifest.origin.sourceDigest='e'.repeat(64);save();expect(()=>pinnedProxy(f.root)).toThrow('provenance');
  }finally{rmSync(f.root,{recursive:true,force:true});}
 });
 it('allows only a source-bound independently reviewed internal proxy and preserves the formal pin',()=>{
  const f=fixture();try{
   const digest=(bytes:Buffer|string)=>createHash('sha256').update(bytes).digest('hex');
   writeFileSync(join(f.base,'src/proxy.rs'),'reviewed new lifecycle');
   expect(()=>selectedProxy(f.root,{},'compiler')).toThrow('source contract');
   const bytes=pe(['DllGetClassObject','DllCanUnloadNow','ReviewedBuild']);
   const hash=validateComponent(bytes,'proxy'),dir=join(f.root,'.local/text-service-build',hash);mkdirSync(dir,{recursive:true});writeFileSync(join(dir,'proxy.dll'),bytes);
   const source=proxySourceIdentity(f.root),reviewFile=join(f.root,'review.json');
   const review={decision:'approved',proxySha256:hash,sourceDigest:digest(JSON.stringify(source)),reviewer:'independent'};
   writeFileSync(reviewFile,JSON.stringify(review));
   const manifest={schemaVersion:1,kind:'reviewed-development-proxy',abi:1,platform:'windows-x86_64-msvc',sha256:hash,size:bytes.length,predecessor:f.manifest.sha256,source,compilerSha256:digest('compiler'),reviewFile,reviewSha256:digest(readFileSync(reviewFile))};
   const manifestFile=join(f.root,'upgrade.json'),save=()=>writeFileSync(manifestFile,JSON.stringify(manifest));save();
   const env={SAYWEAVE_BUILD_CHANNEL:'internal',SAYWEAVE_TSF_PROXY_UPGRADE_MANIFEST:manifestFile};
   expect(selectedProxy(f.root,env,'compiler').path).toBe(join(dir,'proxy.dll'));
   expect(()=>selectedProxy(f.root,{...env,SAYWEAVE_BUILD_CHANNEL:'stable'},'compiler')).toThrow('internal-only');
   expect(()=>selectedProxy(f.root,env,'changed compiler')).toThrow('identity');
   writeFileSync(reviewFile,JSON.stringify({...review,decision:'pending'}));expect(()=>selectedProxy(f.root,env,'compiler')).toThrow('review');
   writeFileSync(reviewFile,JSON.stringify(review));writeFileSync(join(f.base,'src/proxy_transport.rs'),'unreviewed change');expect(()=>selectedProxy(f.root,env,'compiler')).toThrow('identity');
   expect(readFileSync(join(f.resource,'abi1.dll'))).toEqual(f.bytes);
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
