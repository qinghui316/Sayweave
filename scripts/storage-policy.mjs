import {execFileSync} from 'node:child_process';
import {createHash,randomUUID} from 'node:crypto';
import {closeSync,copyFileSync,existsSync,lstatSync,mkdirSync,openSync,readdirSync,readFileSync,renameSync,statfsSync,unlinkSync,writeFileSync} from 'node:fs';
import path from 'node:path';

export const storageLimits=Object.freeze({incrementalBytes:10*1024**3,scratchBytes:12*1024**3,installerBytes:3*1024**3,reportsBytes:2*1024**3,failedRounds:3,failedDays:7,reportBytes:64*1024**2,reserveBytes:2*1024**3});
const uuid='[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}';
const contextPattern=new RegExp(`^\\.local/(?:data-acceptance|windows-update|installed-startup)/${uuid}$`);
const cachePattern=/^src-tauri\/(?:text-service\/)?target\/(?:debug|release)\/incremental\/[a-zA-Z0-9_]+-[a-z0-9]+$/;
export function storagePath(root,relative){
 if(typeof relative!=='string'||relative.includes('\\')||relative.split('/').some(part=>!part||part==='.'||part==='..')||path.isAbsolute(relative))throw Error('Invalid relative storage path');
 const base=path.resolve(root),absolute=path.resolve(base,relative);
 if(!absolute.startsWith(base+path.sep))throw Error('Storage path escaped workspace');
 for(let cursor=absolute;cursor;cursor=path.dirname(cursor)){
  if(existsSync(cursor)&&lstatSync(cursor).isSymbolicLink())throw Error(`Storage link protected: ${relative}`);
  if(path.dirname(cursor)===cursor)break;
 }
 return absolute;
}
export function storageTree(root,relative){
 const absolute=storagePath(root,relative),entries=[];let bytes=0,latest=0;
 if(!existsSync(absolute))return {relative,bytes:0,files:0,latest:0,fingerprint:null,absent:true};
 function walk(directory,prefix=''){
  for(const name of readdirSync(directory).sort()){
   const file=path.join(directory,name),stat=lstatSync(file),key=prefix+name;
   if(stat.isSymbolicLink())throw Error(`Nested storage link protected: ${relative}/${key}`);
   latest=Math.max(latest,stat.mtimeMs);
   if(stat.isDirectory()){entries.push(`D\t${key}`);walk(file,key+'/');}
   else if(stat.isFile()){bytes+=stat.size;entries.push(`F\t${key}\t${stat.size}\t${stat.mtimeMs}`);}
   else throw Error('Unsupported storage file kind');
  }
 }
 walk(absolute);return {relative,bytes,files:entries.filter(line=>line.startsWith('F\t')).length,latest,fingerprint:createHash('sha256').update(entries.join('\n')).digest('hex')};
}
export function writeStorageJson(root,relative,value){
 const file=storagePath(root,relative);mkdirSync(path.dirname(file),{recursive:true});
 const temporary=file+'.'+randomUUID()+'.tmp';writeFileSync(temporary,JSON.stringify(value,null,2)+'\n','utf8');renameSync(temporary,file);
}
function readJson(root,relative,fallback){const file=storagePath(root,relative);return existsSync(file)?JSON.parse(readFileSync(file,'utf8').replace(/^\uFEFF/,'')):fallback;}
export function readPins(root){
 const pins=readJson(root,'.local/storage/pins.json',[]);
 if(!Array.isArray(pins)||pins.some(pin=>!pin.reason||!pin.relative))throw Error('Invalid storage pins');
 for(const pin of pins)storagePath(root,pin.relative);return pins;
}
const protectedBy=(relative,pins)=>pins.some(pin=>{const item=relative.toLowerCase(),protectedPath=pin.relative.toLowerCase();return item===protectedPath||item.startsWith(protectedPath+'/')||protectedPath.startsWith(item+'/');});
export function failedRetention(records,now=Date.now(),limits=storageLimits){
 const failed=records.filter(record=>['failed','cancelled','interrupted'].includes(record.status)).sort((a,b)=>b.finishedAt-a.finishedAt);
 return new Set(failed.filter((record,index)=>!record.pin&&record.cleanupConfirmed&&(index>=limits.failedRounds||now-record.finishedAt>limits.failedDays*86400000)).map(record=>record.id));
}
function ownerAlive(pid){try{process.kill(pid,0);return true;}catch(error){if(error.code==='ESRCH')return false;throw error;}}
function storageLock(root,action){
 const file=storagePath(root,'.local/storage/reservation.lock');mkdirSync(path.dirname(file),{recursive:true});
 if(existsSync(file)){const old=JSON.parse(readFileSync(file,'utf8'));if(ownerAlive(old.pid))throw Error('Another storage reservation is active');unlinkSync(file);}
 const token=randomUUID(),handle=openSync(file,'wx');writeFileSync(handle,JSON.stringify({pid:process.pid,token}));closeSync(handle);
 let value,error;
 try{value=action();}catch(caught){error=caught;}
 if(JSON.parse(readFileSync(file,'utf8')).token!==token)throw Error('Storage reservation ownership changed');unlinkSync(file);
 if(error)throw error;return value;
}
export function planStorage(root,{now=Date.now(),limits=storageLimits}={}){
 const pins=readPins(root),plan={schemaVersion:1,root:path.resolve(root),createdAt:now,limits,delete:[],keep:[],blocked:[]};
 let observed=0,kept=0;const units=[];
 for(const prefix of ['src-tauri/target/debug/incremental','src-tauri/target/release/incremental','src-tauri/text-service/target/debug/incremental','src-tauri/text-service/target/release/incremental']){
  const directory=storagePath(root,prefix);if(!existsSync(directory))continue;
  for(const name of readdirSync(directory)){
   const relative=prefix+'/'+name;if(!cachePattern.test(relative)){plan.blocked.push({relative,reason:'Unknown incremental entry'});continue;}
   try{units.push(storageTree(root,relative));}catch(error){plan.blocked.push({relative,reason:error.message});}
  }
 }
 units.sort((a,b)=>b.latest-a.latest);
 for(const entry of units){
   observed+=entry.bytes;
   if(protectedBy(entry.relative,pins)){kept+=entry.bytes;plan.keep.push({...entry,reason:'pinned'});}
   else if(kept+entry.bytes<=limits.incrementalBytes){kept+=entry.bytes;plan.keep.push({...entry,reason:'recent incremental working set'});}
   else plan.delete.push({...entry,kind:'incremental'});
 }
 const manifestRoot=storagePath(root,'.local/storage/contexts');
 const records=existsSync(manifestRoot)?readdirSync(manifestRoot).filter(name=>new RegExp(`^${uuid}\\.json$`).test(name)).map(name=>readJson(root,'.local/storage/contexts/'+name)):[];
 const expired=failedRetention(records,now,limits);let scratchBytes=0;
 for(const record of records){
  if(!contextPattern.test(record.relative)||record.owner!=='sayweave-validation-v1')throw Error('Invalid owned context');
  try{
   const entry=storageTree(root,record.relative);if(entry.absent&&record.status!=='active')continue;scratchBytes+=record.status==='active'?Math.max(entry.bytes,record.reservationBytes||0):entry.bytes;
   const active=record.status==='active';
   if(active&&!ownerAlive(record.pid))plan.blocked.push({relative:record.relative,reason:'Interrupted owner; cleanup and credential settlement require reconciliation',id:record.id});
   if(active||record.pin||!record.cleanupConfirmed||protectedBy(record.relative,pins))plan.keep.push({...entry,id:record.id,reason:active?'active owner':record.pin||'unsettled cleanup or pinned reference'});
   else if(record.status==='succeeded'||expired.has(record.id))plan.delete.push({...entry,kind:'context',id:record.id});
   else plan.keep.push({...entry,id:record.id,reason:'failed retention window'});
  }catch(error){plan.blocked.push({relative:record.relative,reason:error.message});}
 }
 for(const prefix of ['.local/data-acceptance','.local/windows-update','.local/installed-startup']){
  const directory=storagePath(root,prefix);if(!existsSync(directory))continue;
  for(const name of readdirSync(directory)){
   const relative=prefix+'/'+name;if(!contextPattern.test(relative)||records.some(record=>record.relative===relative))continue;
   try{const entry=storageTree(root,relative);scratchBytes+=entry.bytes;plan.keep.push({...entry,reason:'Legacy environment has no owned settlement record'});}catch(error){plan.blocked.push({relative,reason:error.message});}
  }
 }
 plan.incremental={observedBytes:observed,keptBytes:kept,budgetBytes:limits.incrementalBytes};plan.scratchBytes=scratchBytes;
 return plan;
}
export function assertStorageCapacity(root,requestedBytes=0,{allocation='scratch'}={}){
 if(!Number.isSafeInteger(requestedBytes)||requestedBytes<0)throw Error('Invalid allocation reservation');
 if(!['scratch','installer'].includes(allocation))throw Error('Invalid allocation class');
 const plan=planStorage(root),disk=statfsSync(root),free=Number(disk.bavail)*Number(disk.bsize);
 const installers=storageTree(root,'.local/storage/installers').bytes,reports=storageTree(root,'.local/storage/reports').bytes;
 if(plan.incremental.observedBytes>storageLimits.incrementalBytes||plan.scratchBytes+(allocation==='scratch'?requestedBytes:0)>storageLimits.scratchBytes||installers+(allocation==='installer'?requestedBytes:0)>storageLimits.installerBytes||reports>storageLimits.reportsBytes||free-requestedBytes<storageLimits.reserveBytes||plan.blocked.length)throw Error('Storage budget requires safe cleanup; large allocation refused. Run npm run storage:plan and inspect blocked/pinned entries.');
 return plan;
}
export function beginValidation(root,{id=randomUUID(),relative=`.local/data-acceptance/${id}`,kind,reservationBytes=64*1024**2,ownerPid=process.pid}={}){
 if(!kind||!contextPattern.test(relative)||!relative.endsWith('/'+id))throw Error('Validation context must be an owned isolated UUID');
 if(ownerPid!==process.pid&&ownerPid!==process.ppid)throw Error('External context owner must be the calling parent');
 return storageLock(root,()=>{
 reconcileStorage(root);assertStorageCapacity(root,reservationBytes);
 const manifest=`.local/storage/contexts/${id}.json`;
 if(existsSync(storagePath(root,manifest))||existsSync(storagePath(root,relative)))throw Error('Existing context requires explicit reconciliation');
 const record={schemaVersion:1,owner:'sayweave-validation-v1',id,relative,kind,pid:ownerPid,externalOwner:ownerPid!==process.pid,createdAt:Date.now(),reservationBytes,status:'active',cleanupConfirmed:false,pin:null};
 writeStorageJson(root,manifest,record);mkdirSync(storagePath(root,relative),{recursive:true});return record;
 });
}
export function reconcileStorage(root){
 const directory=storagePath(root,'.local/storage/contexts');if(!existsSync(directory))return [];
 const reconciled=[];
 for(const name of readdirSync(directory).filter(name=>new RegExp(`^${uuid}\\.json$`).test(name))){
  const relative='.local/storage/contexts/'+name,record=readJson(root,relative);
  if(record.owner!=='sayweave-validation-v1'||!contextPattern.test(record.relative))throw Error('Foreign storage context');
  if(record.status==='active'&&!ownerAlive(record.pid)){
   const updated={...record,status:'interrupted',finishedAt:Date.now(),cleanupConfirmed:false,pin:'Interrupted validation: native resources or credential cleanup unconfirmed'};
   writeStorageJson(root,relative,updated);reconciled.push(updated);
  }
 }
 return reconciled;
}
export function applyStoragePlan(root,plan){
 if(process.platform!=='win32')throw Error('Native Windows path/process/handle verification required for cleanup');
 const relative=`.local/storage/plans/${randomUUID()}.json`;writeStorageJson(root,relative,plan);
 const output=execFileSync('powershell',['-NoProfile','-ExecutionPolicy','Bypass','-File',path.join(root,'scripts/storage-safety.ps1'),'-PlanFile',storagePath(root,relative),'-Apply'],{encoding:'utf8',windowsHide:true,maxBuffer:1024*1024});
 return JSON.parse(output.trim());
}
export function finishValidation(root,record,report,{processesClosed=false,credentialsCleared=false,pin=null,apply=true}={}){
 if(record.owner!=='sayweave-validation-v1'||record.pid!==process.pid&&!(record.externalOwner&&record.pid===process.ppid))throw Error('Foreign context settlement refused');
 const saved=readJson(root,`.local/storage/contexts/${record.id}.json`);
 if(saved.status!=='active'||saved.relative!==record.relative||saved.pid!==record.pid||saved.externalOwner!==record.externalOwner)throw Error('Stale context settlement');
 const destination=`.local/storage/reports/${record.id}`;
 if(Buffer.byteLength(JSON.stringify(report))>storageLimits.reportBytes)throw Error('Compact report budget exceeded; context retained');
 writeStorageJson(root,destination+'/report.json',report);
 let retained=Buffer.byteLength(JSON.stringify(report));
 const directory=storagePath(root,record.relative);
 if(existsSync(directory))for(const name of readdirSync(directory).filter(name=>/^[a-zA-Z0-9_.-]+\.png$/.test(name))){
  const file=storagePath(root,record.relative+'/'+name),stat=lstatSync(file);
  if(!stat.isFile())throw Error('Report image must be a regular file');retained+=stat.size;
  if(retained>storageLimits.reportBytes)throw Error('Compact report budget exceeded; context retained');
  copyFileSync(file,storagePath(root,destination+'/'+name));
 }
 const updated={...saved,status:report.passed?'succeeded':'failed',finishedAt:Date.now(),cleanupConfirmed:processesClosed&&credentialsCleared,pin:pin||(!processesClosed||!credentialsCleared?'Process or credential cleanup unconfirmed':null),report:destination};
 writeStorageJson(root,`.local/storage/contexts/${record.id}.json`,updated);
 const plan=planStorage(root);return {record:updated,cleanup:apply?applyStoragePlan(root,{...plan,delete:plan.delete.filter(item=>item.kind==='context')}):plan};
}
export function retainInstaller(root,file,{productCommit,purpose,expectedSha256}={}){
 if(!/^[a-f0-9]{40}$/.test(productCommit)||!purpose||!/^[a-f0-9]{64}$/.test(expectedSha256))throw Error('Installer provenance required');
 const relative=path.relative(path.resolve(root),path.resolve(file)).split(path.sep).join('/');storagePath(root,relative);
 const stat=lstatSync(file);if(!stat.isFile()||stat.size<1||stat.size>256*1024**2)throw Error('Installer size/type differs');
 const bytes=readFileSync(file),signature=readFileSync(storagePath(root,relative+'.sig'));
 if(signature.length<1||signature.length>16384||createHash('sha256').update(bytes).digest('hex')!==expectedSha256)throw Error('Installer bytes differ');
 const prefix=`.local/storage/installers/${expectedSha256}`,installer=storagePath(root,prefix+'/installer.exe');
 if(existsSync(installer)){
  if(createHash('sha256').update(readFileSync(installer)).digest('hex')!==expectedSha256||!readFileSync(storagePath(root,prefix+'/installer.exe.sig')).equals(signature))throw Error('Stored installer identity differs');
 }else{
  assertStorageCapacity(root,bytes.length,{allocation:'installer'});mkdirSync(storagePath(root,prefix),{recursive:true});
  writeFileSync(installer,bytes);writeFileSync(storagePath(root,prefix+'/installer.exe.sig'),signature);
 }
 const receipt=readJson(root,prefix+'/receipt.json',{schemaVersion:1,sha256:expectedSha256,size:bytes.length,signatureSha256:createHash('sha256').update(signature).digest('hex'),uses:[]});
 if(!receipt.uses.some(use=>use.productCommit===productCommit&&use.purpose===purpose&&use.source===relative))receipt.uses.push({productCommit,purpose,source:relative});
 writeStorageJson(root,prefix+'/receipt.json',receipt);return {installer,sha256:expectedSha256,reused:receipt.uses.length>1};
}
