import {mkdtempSync,writeFileSync,readFileSync,rmSync,unlinkSync,linkSync,statSync} from 'node:fs';
import {createHash} from 'node:crypto';
import os from 'node:os';
import path from 'node:path';
import {afterEach,expect,it} from 'vitest';
// @ts-expect-error Build helper is native JavaScript.
import {runtimeNames,validateLocalRuntime,stageRuntimeFile} from '../scripts/build-local-asr.mjs';
// @ts-expect-error Acceptance helper is native JavaScript.
import {nsisPayloadInventory} from '../scripts/windows-update-packages.mjs';
const dirs:string[]=[];
afterEach(()=>{for(const dir of dirs.splice(0))rmSync(dir,{recursive:true,force:true});});
it('freezes independent runtime bytes even when the old candidate was a Cargo hardlink',()=>{
 const dir=mkdtempSync(path.join(os.tmpdir(),'local-runtime-'));dirs.push(dir);
 const from=path.join(dir,'compiler-output.dll'),to=path.join(dir,'runtime.dll');
 const bytes=Buffer.from('compiled native runtime'),identity={bytes:bytes.length,sha256:createHash('sha256').update(bytes).digest('hex')};
 writeFileSync(from,bytes);linkSync(from,to);
 stageRuntimeFile(from,to,identity);
 expect(statSync(to).nlink).toBe(1);
 writeFileSync(from,'next compilation');
 expect(readFileSync(to)).toEqual(bytes);
 expect(()=>stageRuntimeFile(from,to,identity)).not.toThrow();
 expect(readFileSync(to)).toEqual(bytes);
});
it('rejects complete omission or partial installation of the new inference payload',()=>{
 const ordinary=['sayweave.exe','sayweave-installer-helper.exe','uninstall.exe'];
 const temporary=['sayweave-main.exe','sayweave-maintenance.exe','sayweave-uninstall.exe'].map(n=>`$PLUGINSDIR\\${n}`);
 const names=runtimeNames() as string[];
 const listing=(files:string[])=>'Type = Nsis\n\n----------\n'+[...files,...temporary].map(n=>`Path = ${n}\nSize = 1\n`).join('\n');
 const installed=[...ordinary,...names.map(n=>`local-asr\\${n}`)];
 expect(nsisPayloadInventory(listing(installed),{localRuntime:true}).installed).toEqual(installed.sort());
 for(const files of [ordinary,installed.slice(0,-1),[...installed,'local-asr\\injected.dll'],[...installed,installed[3]!]])expect(()=>nsisPayloadInventory(listing(files),{localRuntime:true})).toThrow();
 expect(nsisPayloadInventory(listing(ordinary)).installed).toEqual(ordinary.sort());
 expect(()=>nsisPayloadInventory(listing(installed))).toThrow();
});
it('binds the worker, all backend DLLs and UTF-8 notices to one complete manifest',()=>{
 const dir=mkdtempSync(path.join(os.tmpdir(),'local-runtime-'));dirs.push(dir);
 const files:Record<string,{bytes:number;sha256:string}>={};
 for(const name of runtimeNames() as string[]){
  const data=Buffer.alloc(256);data.write('MZ');data.writeUInt32LE(64,0x3c);data.writeUInt32LE(0x4550,64);data.writeUInt16LE(0x8664,68);
  const bytes=name==='licenses.txt'?Buffer.from('MIT notice\n'.repeat(20)):data;
  writeFileSync(path.join(dir,name),bytes);files[name]={bytes:bytes.length,sha256:createHash('sha256').update(bytes).digest('hex')};
 }
 const manifest={version:1,engine:'0.1.3',target:'x86_64-pc-windows-msvc',files};
 expect(validateLocalRuntime(dir,manifest)).toBe(manifest);
 writeFileSync(path.join(dir,'ggml.dll'),Buffer.alloc(256));expect(()=>validateLocalRuntime(dir,manifest)).toThrow('hash');
 unlinkSync(path.join(dir,'ggml.dll'));expect(()=>validateLocalRuntime(dir,manifest)).toThrow();
 expect(()=>validateLocalRuntime(dir,{...manifest,files:{}})).toThrow('inventory');
});
