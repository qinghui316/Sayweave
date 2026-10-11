// Read-only by default. Native cleanup independently revalidates every allowlisted entry.
import {execFileSync} from 'node:child_process';
import {readFileSync} from 'node:fs';
import {fileURLToPath} from 'node:url';
import path from 'node:path';
import {randomUUID} from 'node:crypto';
import {applyStoragePlan,beginValidation,finishValidation,planStorage,storagePath,storageTree,writeStorageJson} from './storage-policy.mjs';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const [mode='plan',file]=process.argv.slice(2);
if(mode==='plan'){
 const plan=planStorage(root),relative=`.local/storage/plans/${randomUUID()}.json`;writeStorageJson(root,relative,plan);
 console.log(JSON.stringify({plan:relative,incremental:plan.incremental,scratchBytes:plan.scratchBytes,delete:plan.delete.map(item=>({relative:item.relative,bytes:item.bytes,kind:item.kind})),blocked:plan.blocked},null,2));
}else if(mode==='apply'&&file){
 const plan=JSON.parse(readFileSync(storagePath(root,file),'utf8').replace(/^\uFEFF/,''));
 if(plan.root!==root||Date.now()-plan.createdAt>900000)throw Error('Wrong workspace or expired plan');
 const verified=[],skipped=[];
 for(const entry of plan.delete){try{if(storageTree(root,entry.relative).fingerprint!==entry.fingerprint)throw Error('Snapshot changed');verified.push(entry);}catch(error){skipped.push({relative:entry.relative,reason:error.message});}}
 console.log(JSON.stringify({...applyStoragePlan(root,{...plan,delete:verified}),precheckSkipped:skipped},null,2));
}else if(mode==='begin-desktop'&&file){
 console.log(JSON.stringify(beginValidation(root,{id:file,kind:'desktop-acceptance',ownerPid:process.ppid})));
}else if(mode==='finish-desktop'&&file){
 const record=JSON.parse(readFileSync(storagePath(root,`.local/storage/contexts/${file}.json`),'utf8'));
 const succeeded=process.argv[4]==='passed';
 console.log(JSON.stringify(finishValidation(root,record,{passed:succeeded,kind:'desktop-acceptance'},{processesClosed:succeeded,credentialsCleared:true})));
}else if(mode==='verify-root'){
 const gitRoot=execFileSync('git',['rev-parse','--show-toplevel'],{cwd:root,encoding:'utf8'}).trim();
 if(path.resolve(gitRoot)!==root)throw Error('Workspace identity differs');console.log(root);
}else throw Error('Usage: node scripts/storage.mjs [plan | apply .local/storage/plans/UUID.json]');
