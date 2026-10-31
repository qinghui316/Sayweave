// Isolated runner gate using the exact ordinary proxy and release core bytes.
// Never install on an owner machine; no private proxy exports or loader bypass.
import {readFileSync,writeFileSync,mkdirSync,existsSync} from 'node:fs';
import {spawnSync} from 'node:child_process';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {createHash} from 'node:crypto';
import {pinnedProxy,validateComponent,signedProxyBaseline} from './build-text-service.mjs';
const hash=bytes=>createHash('sha256').update(bytes).digest('hex');
export function extractBaseline(main,helper,manifest){
 const origin=signedProxyBaseline(manifest);
 if(hash(main)!==origin.mainSha256||hash(helper)!==origin.helperSha256)throw Error('Baseline executable identity differs');
 const result={};
 for(const [role,identity] of [['proxy',origin.proxy],['core',origin.core]]){
  if(!identity||!Number.isSafeInteger(identity.size)||identity.size<256||identity.size>32*1024*1024)throw Error('Baseline component size invalid');
  const parts=[main,helper].map((bytes,index)=>{
   const offset=identity[index?'helperOffset':'mainOffset'];
   if(!Number.isSafeInteger(offset)||offset<0||offset+identity.size>bytes.length)throw Error('Baseline component offset invalid');
   return bytes.subarray(offset,offset+identity.size);
  });
  if(!parts[0].equals(parts[1])||validateComponent(parts[0],role)!==identity.sha256)throw Error('Baseline embedded component differs');
  result[role]=parts[0];
 }
 return result;
}
export function validateFormalEvidence(evidence,manifest,coreHash){
 const baseline=signedProxyBaseline(manifest),changed=baseline.proxy.sha256!==manifest.sha256;
 if(!evidence?.formalProxySwitchPassed)throw Error('Formal switch evidence missing');
 if(!changed)return;
 const old=evidence.oldTarget,fresh=evidence.newTarget,coreChanged=coreHash!==baseline.core.sha256;
 if(!evidence.proxyUpgradePassed||!evidence.oldTargetExited||!evidence.newTargetExited||evidence.coreChanged!==coreChanged
   ||!old?.formalTargetPassed||!fresh?.formalTargetPassed||!old.cleanupComplete||!fresh.cleanupComplete||!fresh.moduleUnloadReady
   ||typeof old.moduleUnloadReady!=='boolean'||old.phase!=='old'||fresh.phase!=='new'
   ||!Number.isInteger(old.pid)||!Number.isInteger(fresh.pid)||old.pid===fresh.pid
   ||old.oldAdmissionRejected!=='enginePrepare:80070005'||old.finalBody!=='旧核心输入。'
   ||fresh.finalBody!==(coreChanged?'旧核心输入。新核心输入。':'旧核心输入。')
   ||old.records?.length!==1||fresh.records?.length!==(coreChanged?2:1))throw Error('Proxy upgrade body or process evidence incomplete');
 for(const [target,proxy,cores] of [[old,baseline.proxy.sha256,[baseline.core.sha256]],[fresh,manifest.sha256,coreChanged?[baseline.core.sha256,coreHash]:[coreHash]]]){
  if(target.processIdentity?.pid!==target.pid||!target.sameService||!target.ordinaryTransport)throw Error('Target identity evidence incomplete');
  target.records.forEach((record,index)=>{
   const runtime=record.runtime;
   if(runtime?.process!==target.pid||runtime.proxy?.sha256!==proxy||runtime.core?.sha256!==cores[index]
     ||typeof runtime.engine?.proxy_instance!=='string'||!runtime.engine.proxy_instance
     ||!Number.isSafeInteger(runtime.engine.engine_generation)||runtime.engine.engine_generation<=0)
     throw Error('Loaded component identity differs');
  });
 }
 if(coreChanged){
  const [a,b]=fresh.records.map(record=>record.runtime);
  if(a.engine.proxy_instance!==b.engine.proxy_instance||a.engine.engine_generation>=b.engine.engine_generation||a.proxy.base!==b.proxy.base)
   throw Error('New proxy instance or core generation differs');
 }
}
function run(file,args,output){
 const result=spawnSync(file,args,{encoding:'utf8',windowsHide:true,maxBuffer:16*1024*1024,timeout:path.basename(file)==='tsf-probe.exe'?120000:1200000});
 if(output)writeFileSync(output,`${result.stdout??''}\n${result.stderr??''}`);
 if(result.error)throw result.error;
 if(result.status!==0)throw Error(`${path.basename(file)} failed: ${result.stderr||result.stdout}`);
 return result.stdout;
}
export async function gate(){
 if(process.env.GITHUB_ACTIONS!=='true'||process.env.RUNNER_ENVIRONMENT!=='github-hosted')throw Error('Disposable GitHub-hosted runner required; local deployment forbidden');
 const pinned=pinnedProxy(process.cwd()),origin=signedProxyBaseline(pinned.manifest),core=process.env.SAYWEAVE_TSF_PAYLOAD;
 if(!core)throw Error('Ordinary core required');
 const coreHash=validateComponent(readFileSync(core),'core');
 const proxyChanged=origin.proxy.sha256!==pinned.manifest.sha256;
 if(!proxyChanged&&coreHash===origin.core.sha256)throw Error('Ordinary component bytes must really change');
 const directory=path.resolve('.local/formal-core-switch');mkdirSync(directory,{recursive:true});
 const installer=path.join(directory,origin.installer);
 const url=`https://github.com/${origin.repository}/releases/download/${origin.tag}/${origin.installer}`;
 for(const [suffix,digest] of [['',origin.installerSha256],['.sig',null]]){
  const response=await fetch(url+suffix,{signal:AbortSignal.timeout(120000)});if(!response.ok)throw Error(`Baseline download ${response.status}`);
  const bytes=Buffer.from(await response.arrayBuffer());if(digest&&hash(bytes)!==digest)throw Error('Baseline package changed');
  writeFileSync(installer+suffix,bytes);
 }
 const tool=path.join(process.env.SAYWEAVE_VERIFIER_DIR??'','verify-update-signature.exe');
 if(!existsSync(tool))throw Error('Existing full native verifier required');
 const config=JSON.parse(readFileSync('src-tauri/tauri.conf.json','utf8'));
 run(tool,[config.plugins.updater.pubkey,installer,installer+'.sig',origin.tag.slice(1)],path.join(directory,'baseline-signature.log'));
 run('7z',['x',installer,`-o${directory}`,'sayweave.exe','sayweave-installer-helper.exe','-y']);
 const baseline=extractBaseline(readFileSync(path.join(directory,'sayweave.exe')),readFileSync(path.join(directory,'sayweave-installer-helper.exe')),pinned.manifest);
 const old=path.join(directory,'old-core.dll');writeFileSync(old,baseline.core);
 const oldProxy=path.join(directory,'old-proxy.dll');writeFileSync(oldProxy,baseline.proxy);
 // Probe compilation does not rebuild/replace the supplied ordinary release core.
 run('cargo',['build','--locked','--manifest-path','src-tauri/text-service/Cargo.toml','--features','acceptance','--bin','tsf-probe'],path.join(directory,'probe-build.log'));
 const output=run(path.resolve('src-tauri/text-service/target/debug/tsf-probe.exe'),[proxyChanged?'formal-proxy-upgrade':'formal-core-switch',pinned.path,old,core,...(proxyChanged?[oldProxy]:[])],path.join(directory,'body-receipts.log'));
 const evidence=output.split(/\r?\n/).filter(line=>line.startsWith('{')).map(line=>JSON.parse(line)).find(value=>value.formalProxySwitchPassed);
 if(!evidence||!output.includes('cleanup_complete=true')||(proxyChanged&&!evidence.proxyUpgradePassed))throw Error('Formal component switch proof incomplete');
 validateFormalEvidence(evidence,pinned.manifest,coreHash);
 writeFileSync(path.join(directory,'receipt.json'),JSON.stringify({schemaVersion:2,origin,proxyChanged,coreChanged:coreHash!==origin.core.sha256,proxySha256:pinned.manifest.sha256,newCoreSha256:coreHash,evidence},null,2));
 console.log(`Formal proxy switch passed: ${pinned.manifest.sha256}`);
}
if(process.argv[1]&&path.resolve(process.argv[1])===fileURLToPath(import.meta.url))await gate();
