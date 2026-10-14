// Isolated runner gate using the exact ordinary proxy and release core bytes.
// Never install on an owner machine; no private proxy exports or loader bypass.
import {readFileSync,writeFileSync,mkdirSync,existsSync} from 'node:fs';
import {spawnSync} from 'node:child_process';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {createHash} from 'node:crypto';
import {pinnedProxy,validateComponent} from './build-text-service.mjs';
const hash=bytes=>createHash('sha256').update(bytes).digest('hex');
export function extractBaseline(main,helper,manifest){
 const origin=manifest.origin;
 if(hash(main)!==origin.mainSha256||hash(helper)!==origin.helperSha256)throw Error('Baseline executable identity differs');
 const result={};
 for(const [role,identity] of [['proxy',{...manifest,...origin}],['core',origin.core]]){
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
function run(file,args,output){
 const result=spawnSync(file,args,{encoding:'utf8',windowsHide:true,maxBuffer:16*1024*1024,timeout:path.basename(file)==='tsf-probe.exe'?120000:1200000});
 if(output)writeFileSync(output,`${result.stdout??''}\n${result.stderr??''}`);
 if(result.error)throw result.error;
 if(result.status!==0)throw Error(`${path.basename(file)} failed: ${result.stderr||result.stdout}`);
 return result.stdout;
}
export async function gate(){
 if(process.env.GITHUB_ACTIONS!=='true'||process.env.RUNNER_ENVIRONMENT!=='github-hosted')throw Error('Disposable GitHub-hosted runner required; local deployment forbidden');
 const pinned=pinnedProxy(process.cwd()),origin=pinned.manifest.origin,core=process.env.SAYWEAVE_TSF_PAYLOAD;
 if(!core||validateComponent(readFileSync(core),'core')===origin.core.sha256)throw Error('New ordinary core required');
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
 // Probe compilation does not rebuild/replace the supplied ordinary release core.
 run('cargo',['build','--locked','--manifest-path','src-tauri/text-service/Cargo.toml','--features','acceptance','--bin','tsf-probe'],path.join(directory,'probe-build.log'));
 const output=run(path.resolve('src-tauri/text-service/target/debug/tsf-probe.exe'),['formal-core-switch',pinned.path,old,core],path.join(directory,'body-receipts.log'));
 const evidence=output.split(/\r?\n/).filter(line=>line.startsWith('{')).map(line=>JSON.parse(line)).find(value=>value.formalProxySwitchPassed);
 if(!evidence||!output.includes('cleanup_complete=true'))throw Error('Formal core switch proof incomplete');
 writeFileSync(path.join(directory,'receipt.json'),JSON.stringify({schemaVersion:1,origin,proxySha256:pinned.manifest.sha256,newCoreSha256:hash(readFileSync(core)),evidence},null,2));
 console.log(`Formal proxy switch passed: ${pinned.manifest.sha256}`);
}
if(process.argv[1]&&path.resolve(process.argv[1])===fileURLToPath(import.meta.url))await gate();
