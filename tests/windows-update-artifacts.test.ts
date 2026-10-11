import {createHash} from 'node:crypto';
import {mkdtempSync,readFileSync,writeFileSync,rmSync,symlinkSync,mkdirSync,openSync,ftruncateSync,closeSync} from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {afterEach,describe,expect,it} from 'vitest';
// @ts-expect-error JavaScript acceptance module has no declaration file.
import {readTools,writeTools,readMechanismEvidence,writeMechanismEvidence,proofMarkers} from '../scripts/windows-update-artifacts.mjs';
// @ts-expect-error JavaScript release entry point has no declaration file.
import {findMechanismRun} from '../scripts/reuse-windows-update-evidence.mjs';

const commit='a'.repeat(40),directories:string[]=[];
function directory(){const value=mkdtempSync(path.join(os.tmpdir(),'sayweave-evidence-'));directories.push(value);return value;}
function tools(){return writeTools(directory(),commit,Buffer.from('production DLL'),Buffer.from('acceptance probe'),{rustc:'rustc 1.92',cargo:'cargo 1.92',capabilities:{acceptance:true,installedFixture:true,protocol:8},markers:proofMarkers,registrationClean:true});}
function evidence(){const t=tools(),dir=directory();writeMechanismEvidence(dir,commit,t,{synthetic:true},{passed:true,kind:'mechanism',commit,tools:t.manifest});return dir;}
afterEach(()=>{for(const dir of directories.splice(0))rmSync(dir,{recursive:true,force:true});});
describe('manual release mechanism evidence selection',()=>{
 const run=(databaseId:number,event='workflow_dispatch')=>({databaseId,event,headSha:commit,headBranch:'main',status:'completed',conclusion:'success'});
 const inventory=(artifacts:{name:string;expired:boolean}[])=>({total_count:artifacts.length,artifacts});
 const live=[{name:'windows-update-evidence',expired:false}];
 it('skips a newer successful CI-only run and reuses exact manual full evidence',()=>{
  expect(findMechanismRun([run(2,'push'),run(1)],commit,(id:number)=>inventory(id===1?live:[])).databaseId).toBe(1);
 });
 it('retains successful legacy main push evidence',()=>expect(findMechanismRun([run(1,'push')],commit,()=>inventory(live)).databaseId).toBe(1));
 it.each([{headSha:'b'.repeat(40)},{headBranch:'feature'},{event:'pull_request'},{conclusion:'failure'},{databaseId:0}])('rejects a wrong run identity %j',delta=>{
  expect(()=>findMechanismRun([{...run(1),...delta}],commit,()=>{throw Error('must not read rejected identity');})).toThrow('exact-commit');
 });
 it('rejects CI-only or expired evidence',()=>{
  expect(()=>findMechanismRun([run(1)],commit,()=>inventory([]))).toThrow('exact-commit');
  expect(()=>findMechanismRun([run(1)],commit,()=>inventory([{name:'windows-update-evidence',expired:true}]))).toThrow('exact-commit');
 });
 it('rejects duplicate or truncated inventories',()=>{
  expect(()=>findMechanismRun([run(1)],commit,()=>inventory([...live,...live]))).toThrow('Ambiguous');
  expect(()=>findMechanismRun([run(1)],commit,()=>({total_count:2,artifacts:live}))).toThrow('inventory');
 });
});
describe('immutable Windows acceptance artifact reuse',()=>{
 it('reuses the same source, DLL, probe and completed mechanism receipt',()=>{const dir=evidence(),result=readMechanismEvidence(dir,commit);expect(result.receipt.passed).toBe(true);expect(result.tools.manifest.commit).toBe(commit);expect(result.seed).toEqual({synthetic:true});});
 it('rejects a different candidate and an unproven probe',()=>{const t=tools();expect(()=>readTools(t.directory,'b'.repeat(40))).toThrow('identity');const m=t.manifest;m.proof.markers=[];writeFileSync(path.join(t.directory,'tools.json'),JSON.stringify(m));expect(()=>readTools(t.directory,commit)).toThrow('proof');});
 it.each(['sayweave_text_service.dll','tsf-probe.exe'])('rejects overwritten %s after packaging',name=>{const t=tools();writeFileSync(path.join(t.directory,name),'overwritten');expect(()=>readTools(t.directory,commit)).toThrow('hash');});
 it.each(['legacy-seed.json','mechanism-receipt.json'])('rejects corrupted %s',name=>{const dir=evidence();writeFileSync(path.join(dir,name),'{}');expect(()=>readMechanismEvidence(dir,commit)).toThrow('hash');});
 it.each([{passed:false},{kind:'tools'},{cleanupErrors:[{stage:'trust removal'}]}])('does not publish incomplete mechanism evidence %j',invalid=>{const t=tools();expect(()=>writeMechanismEvidence(directory(),commit,t,{}, {passed:true,kind:'mechanism',commit,tools:t.manifest,...invalid})).toThrow('incomplete');});
 it('rejects a failed receipt even if its checksum was recomputed',()=>{const dir=evidence(),receiptPath=path.join(dir,'mechanism-receipt.json');const receipt=JSON.parse(readFileSync(receiptPath,'utf8'));receipt.passed=false;const bytes=Buffer.from(JSON.stringify(receipt));writeFileSync(receiptPath,bytes);const manifestPath=path.join(dir,'evidence.json'),manifest=JSON.parse(readFileSync(manifestPath,'utf8'));manifest.files['mechanism-receipt.json']=createHash('sha256').update(bytes).digest('hex');writeFileSync(manifestPath,JSON.stringify(manifest));expect(()=>readMechanismEvidence(dir,commit)).toThrow('incomplete');});
 it('rejects a junction that redirects the nested tool directory',()=>{const dir=evidence(),external=directory();rmSync(path.join(dir,'tools'),{recursive:true});symlinkSync(external,path.join(dir,'tools'),'junction');expect(()=>readMechanismEvidence(dir,commit)).toThrow('link');rmSync(path.join(dir,'tools'));});
 it('rejects large files before loading them and rejects directories as binaries',()=>{const t=tools(),dll=path.join(t.directory,'sayweave_text_service.dll');const fd=openSync(dll,'w');try{ftruncateSync(fd,128*1024*1024+1);}finally{closeSync(fd);}expect(()=>readTools(t.directory,commit)).toThrow('bound');rmSync(dll);mkdirSync(dll);expect(()=>readTools(t.directory,commit)).toThrow('not a file');});
});
