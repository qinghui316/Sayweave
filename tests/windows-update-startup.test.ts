import {mkdtempSync,writeFileSync,readFileSync,rmSync} from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {EventEmitter} from 'node:events';
import {spawnSync} from 'node:child_process';
import {afterEach,describe,expect,it,vi} from 'vitest';
// @ts-expect-error JavaScript acceptance module has no declaration file.
import {boundedTail,safeStartupText,pageReady,validateEndpoint,installedStartup,launchEnvironment,restartAndReconnect,aclPrelude,aclRestoreScript} from '../scripts/windows-update-startup.mjs';
// @ts-expect-error JavaScript acceptance module has no declaration file.
import {readPackages,writePackages,nsisExecutableIdentity} from '../scripts/windows-update-packages.mjs';
// @ts-expect-error JavaScript acceptance module has no declaration file.
import {validateDebugPolicyState,debugPolicyScript,prepareDisposableDebugPolicy} from '../scripts/windows-update-debug-policy.mjs';

const directories:string[]=[],commit='a'.repeat(40),uuid='23456789-1234-4321-8123-123456789012';
const context={identifier:`local.sayweave.acceptance.${uuid}`,product:`Sayweave 验收 ${uuid}`,port:19443,publicKey:'public test key '.repeat(4),componentSha256:'b'.repeat(64)};
function directory(){const dir=mkdtempSync(path.join(os.tmpdir(),'sayweave-startup-'));directories.push(dir);return dir;}
function packages(){const input=directory(),file=path.join(input,'setup.exe');writeFileSync(file,'same installer');writeFileSync(`${file}.sig`,'same signature');const output=directory();writePackages(output,commit,context,{old:{installer:file,executableSha256:'c'.repeat(64)}},{passed:true});return output;}
afterEach(()=>{for(const dir of directories.splice(0))rmSync(dir,{recursive:true,force:true});});
describe('installed startup diagnostics and immutable package reuse',()=>{
 it.skipIf(process.platform!=='win32')('accepts only the documented ACL bit addition and rejects permission or identity changes without filesystem operations',()=>{
  const encode=String.raw`$sd=[System.Security.AccessControl.RawSecurityDescriptor]::new('O:BAG:BAD:(A;;FA;;;BA)(A;;FR;;;BU)');$bytes=[byte[]]::new($sd.BinaryLength);$sd.GetBinaryForm($bytes,0);[Convert]::ToBase64String($bytes)`;
  const baseline=spawnSync('powershell',['-NoProfile','-Command',encode],{encoding:'utf8',windowsHide:true,timeout:10000});expect(baseline.status).toBe(0);
  const original=Buffer.from(baseline.stdout.trim(),'base64');
  const cases=[
   {name:'same',change:'',accepted:true},
   {name:'AI addition',change:'$sd.SetFlags($sd.ControlFlags -bor 1024)',accepted:true},
   {name:'AI removal',beforeFlags:1024,change:'$sd.SetFlags($sd.ControlFlags -band (-bnot 1024))',accepted:false},
   {name:'protection flag',change:'$sd.SetFlags($sd.ControlFlags -bor 4096)',accepted:false},
   {name:'owner',change:"$sd.Owner=[System.Security.Principal.SecurityIdentifier]::new('S-1-5-18')",accepted:false},
   {name:'group',change:"$sd.Group=[System.Security.Principal.SecurityIdentifier]::new('S-1-5-18')",accepted:false},
   {name:'ACE principal',change:"$sd=[System.Security.AccessControl.RawSecurityDescriptor]::new('O:BAG:BAD:(A;;FA;;;SY)(A;;FR;;;BU)')",accepted:false},
   {name:'ACE permissions',change:"$sd=[System.Security.AccessControl.RawSecurityDescriptor]::new('O:BAG:BAD:(A;;FR;;;BA)(A;;FR;;;BU)')",accepted:false},
   {name:'ACE order',change:"$sd=[System.Security.AccessControl.RawSecurityDescriptor]::new('O:BAG:BAD:(A;;FR;;;BU)(A;;FA;;;BA)')",accepted:false},
   {name:'resource manager',beforeFlags:16384,beforeManager:9,change:'$sd.ResourceManagerControl=7',accepted:false},
  ];
  for(const value of cases){
   const bytes=Buffer.from(original);bytes.writeUInt16LE(bytes.readUInt16LE(2)|(value.beforeFlags??0),2);bytes[1]=value.beforeManager??0;const binary=bytes.toString('base64');
   const fixture=aclPrelude+String.raw`Add-Type -TypeDefinition 'public class MemoryAcl { private byte[] current; public MemoryAcl(byte[] data){current=data;} public byte[] GetSecurityDescriptorBinaryForm(){return current;} public void SetSecurityDescriptorBinaryForm(byte[] data){} }';$sd=[System.Security.AccessControl.RawSecurityDescriptor]::new([Convert]::FromBase64String('`+binary+String.raw`'),0);`+value.change+String.raw`;$script:fixtureBytes=[byte[]]::new($sd.BinaryLength);$sd.GetBinaryForm($script:fixtureBytes,0);function Get-Acl {param($LiteralPath);[MemoryAcl]::new($script:fixtureBytes)};function Set-Acl {param($LiteralPath,$AclObject)};if((Get-Command Get-Acl).CommandType -ne 'Function' -or (Get-Command Set-Acl).CommandType -ne 'Function'){throw 'Memory-only ACL interception missing'};`;
   const result=spawnSync('powershell',['-NoProfile','-Command',fixture+aclRestoreScript(String.raw`C:\sayweave-pure-acl-no-io`,{binary})],{encoding:'utf8',windowsHide:true,timeout:10000});
   expect(result.status===0,value.name+result.stderr).toBe(value.accepted);const output=JSON.parse(result.stdout.trim());expect(output.descriptorMatches,value.name).toBe(value.accepted);
   if(!value.accepted)expect(result.stderr).toContain('unsupported control bit');
   if(value.name==='resource manager')expect(output.resourceManagerControlMatches).toBe(false);
  }
 },30000);
 it('accepts renderer closure only after a genuinely new process is ready',async()=>{
  const original={ProcessId:42,CreationDate:'2026-10-10T10:00:00Z',ExecutablePath:'owned.exe'},next={...original,ProcessId:43,CreationDate:'2026-10-10T10:00:01Z'},record:Record<string,unknown>[]=[];
  const reconnect=vi.fn(async()=> 'new page');
  const input={original,invoke:async()=>{throw Error('page.evaluate: Target page, context or browser has been closed');},disconnect:async()=>{},inspect:()=>[next],reconnect,current:()=>next,record,persist:()=>{},timeoutMs:2,intervalMs:1};
  expect(await restartAndReconnect(input)).toBe('new page');expect(record[0]).toMatchObject({passed:true,rendererClosed:true,originalEnded:true,next});
  await expect(restartAndReconnect({...input,inspect:()=>[original]})).rejects.toThrow('did not exit');
  await expect(restartAndReconnect({...input,current:()=>original})).rejects.toThrow('new owned');
  reconnect.mockClear();await expect(restartAndReconnect({...input,invoke:async()=>{throw Error('invalidState');}})).rejects.toThrow('invalidState');expect(reconnect).not.toHaveBeenCalled();
 });
 it('binds signed NSIS payload bytes instead of the restored build-output hash',()=>{const source=String.raw`D:\candidate\release\sayweave.exe`,script=`!define MAINBINARYSRCPATH "${source}"\r\n`,original=Buffer.from('MZ-before-__TAURI_BUNDLE_TYPE_VAR_UNK-after'),payload=Buffer.from('MZ-before-__TAURI_BUNDLE_TYPE_VAR_NSS-after');const identity=nsisExecutableIdentity(script,source,original,payload);expect(identity.restoredBuildSha256).not.toBe(identity.executableSha256);expect(identity.executableSize).toBe(payload.length);expect({...{size:999},...identity}.size).toBe(999);expect(identity.mainBinarySource).toBe(source);expect(payload.toString()).toContain('VAR_NSS');const tampered=Buffer.from(payload);tampered[0]=0;expect(()=>nsisExecutableIdentity(script,source,original,tampered)).toThrow('beyond');expect(()=>nsisExecutableIdentity(script,String.raw`D:\other\sayweave.exe`,original,payload)).toThrow('source differs');expect(()=>nsisExecutableIdentity(script+script,source,original,payload)).toThrow('ambiguous');});
 it('refuses existing per-app or wildcard debug policy and running hosts',()=>{const fresh={present:[true,false,false],appPresent:false,wildcardPresent:false,running:0};expect(validateDebugPolicyState(fresh)).toBe(fresh);for(const conflict of [{appPresent:true},{wildcardPresent:true},{running:1},{present:[false,true,false]},{present:[true]},{running:-1}])expect(()=>validateDebugPolicyState({...fresh,...conflict})).toThrow();});
 it('requires a unique ownership identity before any policy mutation or cleanup',()=>{const fresh={present:[true,false,false],appPresent:false,wildcardPresent:false,running:0};for(const operation of ['install','remove'])for(const lease of [null,'',String.raw`wrong';throw 'injected`])expect(()=>debugPolicyScript(operation,9291,fresh,lease)).toThrow('Unique');});
 it('refuses invalid ports before registry access and never grants owner-host permission',()=>{for(const port of [0,80,65536,NaN,9291.5])expect(()=>debugPolicyScript('read',port)).toThrow();vi.stubEnv('GITHUB_ACTIONS','false');vi.stubEnv('SAYWEAVE_DISPOSABLE_WINDOWS','');try{expect(()=>prepareDisposableDebugPolicy(9291)).toThrow('disposable Windows runner');}finally{vi.unstubAllEnvs();}});
 it('removes case-alias environment keys before requesting the owned debug port on Windows',()=>{const source={Path:'system path',WebView2_Additional_Browser_Arguments:'old arguments',WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:'other arguments'};const env=launchEnvironment(source,9291);expect(Object.keys(env).filter(key=>key.toUpperCase()==='WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS')).toEqual(['WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS']);expect(env.WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS).toBe('--remote-debugging-port=9291 --remote-debugging-address=127.0.0.1');expect(env.Path).toBe(source.Path);expect(source.WebView2_Additional_Browser_Arguments).toBe('old arguments');});
 it('records and fails early on actual stream closure instead of waiting for the debug deadline',async()=>{const dir=directory(),exe=path.join(dir,'synthetic.exe');writeFileSync(exe,'synthetic executable');const child=Object.assign(new EventEmitter(),{pid:412,stdout:new EventEmitter(),stderr:new EventEmitter()}),records:Record<string,unknown>[]=[];
  await expect(installedStartup({exe,dataRoot:dir,directory:dir,port:9291,record:records,persist:()=>{},onOwned:()=>{},onSpawn:()=>{},timeoutMs:60000,io:{inspect:()=>({applications:[],webviews:[]}),occupied:async()=>false,spawn:()=>{queueMicrotask(()=>{child.stdout.emit('data',Buffer.from('startup setup'));child.stderr.emit('data',Buffer.from('runtime failed: synthetic WebView failure'));child.emit('close',101,null);});return child;}}})).rejects.toThrow('exit=101');
  expect(records[0]).toMatchObject({pid:412,exitCode:101,phase:'process',passed:false});expect(records[0]!.elapsedMs).toBeLessThan(5000);expect(readFileSync(path.join(dir,'startup-1.stderr.log'),'utf8')).toContain('synthetic WebView failure');expect(readFileSync(path.join(dir,'startup-1.stdout.log'),'utf8')).toContain('startup setup');
 });
 it('bounds noisy startup streams and redacts credentials',()=>{const tail=boundedTail(Buffer.alloc(65536,65),Buffer.from('last stderr'));expect(tail.length).toBe(65536);expect(tail.toString().endsWith('last stderr')).toBe(true);expect(safeStartupText('GH secret-value Bearer abc token=abcd',['secret-value'])).toBe('GH [redacted] Bearer [redacted] token=[redacted]');});
 it('requires a page-ready marker from this process lifetime',()=>{expect(pageReady('operation=startup phase=main_window_created pid=42 at_ms=1000 ',42)).toBe(false);expect(pageReady('operation=startup phase=main_page_ready pid=41 at_ms=1000 ',42)).toBe(false);expect(pageReady('operation=startup phase=main_page_ready pid=42 at_ms=1000 version=0.1.700',42,1000)).toBe(true);expect(pageReady('operation=startup phase=main_page_ready pid=42 at_ms=999 version=0.1.700',42,1000)).toBe(false);});
 it('accepts only the owned loopback browser endpoint',()=>{expect(validateEndpoint({webSocketDebuggerUrl:'ws://127.0.0.1:9291/devtools/browser/test'},9291)).toContain('test');for(const url of ['wss://127.0.0.1:9291/devtools/browser/test','ws://example.com:9291/devtools/browser/test','ws://127.0.0.1:9292/devtools/browser/test','ws://127.0.0.1:9291/devtools/page/test'])expect(()=>validateEndpoint({webSocketDebuggerUrl:url},9291)).toThrow('escaped');});
 it('preserves one partial package for diagnostics without inventing a target package',()=>{const result=readPackages(packages(),commit);expect(result.next).toBeNull();expect(result.manifest.commit).toBe(commit);expect(result.manifest.url).toBe('https://localhost:19443/latest.json');expect(result.entries.old.executableSha256).toBe('c'.repeat(64));});
 it.each(['test-0.1.700-setup.exe','test-0.1.700-setup.exe.sig'])('rejects overwritten artifact %s',name=>{const dir=packages();writeFileSync(path.join(dir,name),'changed');expect(()=>readPackages(dir,commit)).toThrow('hash');});
 it('rejects another commit or untrusted build context',()=>{const dir=packages();expect(()=>readPackages(dir,'d'.repeat(40))).toThrow('identity');const file=path.join(dir,'packages.json'),manifest=JSON.parse(readFileSync(file,'utf8'));manifest.url='https://example.com/latest.json';writeFileSync(file,JSON.stringify(manifest));expect(()=>readPackages(dir,commit)).toThrow('context');});
 it('rejects inventory paths outside the package set',()=>{const dir=packages(),file=path.join(dir,'packages.json'),manifest=JSON.parse(readFileSync(file,'utf8'));manifest.artifacts.old.file='../setup.exe';writeFileSync(file,JSON.stringify(manifest));expect(()=>readPackages(dir,commit)).toThrow('metadata');});
});
