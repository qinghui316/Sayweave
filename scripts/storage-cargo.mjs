import {spawn} from 'node:child_process';
import {existsSync,readFileSync} from 'node:fs';
import {fileURLToPath} from 'node:url';
import path from 'node:path';
import {applyStoragePlan,assertStorageCapacity,planStorage,writeStorageJson} from './storage-policy.mjs';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..'),args=process.argv.slice(2);
if(!args.length)throw Error('Cargo arguments required');
function settleIncremental(){
 const plan=planStorage(root);
 if(plan.incremental.observedBytes>plan.limits.incrementalBytes){
  const result=applyStoragePlan(root,{...plan,delete:plan.delete.filter(item=>item.kind==='incremental')});
  console.log(`Incremental cleanup: ${result.deleted.length} deleted, ${result.skipped.length} skipped, ${result.releasedBytes} bytes reclaimed`);
 }
}
settleIncremental();assertStorageCapacity(root);
const start=Date.now(),child=spawn('cargo',args,{cwd:root,windowsHide:true,stdio:'inherit'});
let spawnError;child.once('error',error=>{spawnError=error;});
const code=await new Promise(resolve=>child.once('close',resolve));
if(spawnError)throw spawnError;
const relative='.local/storage/cargo-history.json',history=existsSync(path.join(root,relative))?JSON.parse(readFileSync(path.join(root,relative),'utf8')):[];
history.push({at:new Date().toISOString(),args,elapsedMs:Date.now()-start,exitCode:code,devDebug:process.env.CARGO_PROFILE_DEV_DEBUG??1,testDebug:process.env.CARGO_PROFILE_TEST_DEBUG??1});
writeStorageJson(root,relative,history.slice(-30));
settleIncremental();process.exitCode=code??1;
