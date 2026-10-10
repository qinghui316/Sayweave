import {createHash} from 'node:crypto';
import {mkdirSync,readFileSync,writeFileSync,lstatSync,realpathSync,existsSync,rmSync} from 'node:fs';
import path from 'node:path';

const names={component:'sayweave_text_service.dll',probe:'tsf-probe.exe'};
const binaryLimit=128*1024*1024,jsonLimit=1024*1024;
export const proofMarkers=['owned production DLL fault export absent=true','owned installed production DLL actual text=true protocol8=true','owned COM DLL unloaded with no live leases=true'];
const hash=bytes=>createHash('sha256').update(bytes).digest('hex');
function identity(commit){if(!/^[a-f0-9]{40}$/.test(commit))throw Error('Exact artifact commit required');}
function root(directory){
 const info=lstatSync(directory);
 if(!info.isDirectory()||info.isSymbolicLink())throw Error('Artifact directory must be a real directory');
 return realpathSync(directory);
}
function target(directory,name){
 const base=root(directory),parts=name.split('/');
 if(parts.some(part=>!part||part==='.'||part==='..'||part.includes('\\'))||path.isAbsolute(name))throw Error('Artifact path escaped its directory');
 let current=base;
 for(const part of parts){current=path.join(current,part);if(existsSync(current)&&lstatSync(current).isSymbolicLink())throw Error('Artifact path contains a link');}
 return current;
}
function file(directory,name,limit=jsonLimit){
 const item=target(directory,name),info=lstatSync(item);
 if(!info.isFile()||info.size<1||info.size>limit)throw Error('Artifact file exceeds bound or is not a file');
 const bytes=readFileSync(item);if(bytes.length!==info.size)throw Error('Artifact changed during read');return bytes;
}
function write(directory,name,bytes,limit=jsonLimit){
 if(!bytes.length||bytes.length>limit)throw Error('Artifact exceeds bound');
 writeFileSync(target(directory,name),bytes);
}
function checkProof(proof){
 if(proof?.registrationClean!==true||proof.capabilities?.acceptance!==true||proof.capabilities.installedFixture!==true||proof.capabilities.protocol!==8||proof.markers?.join()!==proofMarkers.join()||typeof proof.rustc!=='string'||!proof.rustc.startsWith('rustc ')||typeof proof.cargo!=='string'||!proof.cargo.startsWith('cargo '))throw Error('Tooling proof is incomplete');
}
export function writeTools(directory,commit,component,probe,proof){
 identity(commit);checkProof(proof);mkdirSync(directory,{recursive:true});root(directory);
 const manifest={schemaVersion:1,kind:'sayweave-update-tools',commit,target:'windows-x86_64',proof,artifacts:{}};
 for(const [kind,bytes] of [['component',component],['probe',probe]]){
  write(directory,names[kind],bytes,binaryLimit);
  manifest.artifacts[kind]={file:names[kind],size:bytes.length,sha256:hash(bytes)};
 }
 write(directory,'tools.json',Buffer.from(JSON.stringify(manifest,null,2)+'\n'));
 return readTools(directory,commit);
}
export function readTools(directory,commit){
 identity(commit);const manifest=JSON.parse(file(directory,'tools.json'));checkProof(manifest.proof);
 if(manifest.schemaVersion!==1||manifest.kind!=='sayweave-update-tools'||manifest.commit!==commit||manifest.target!=='windows-x86_64'||Object.keys(manifest.artifacts??{}).sort().join()!=='component,probe')throw Error('Tool artifact identity differs');
 for(const [kind,name] of Object.entries(names)){
  const item=manifest.artifacts[kind];
  if(item.file!==name||!Number.isSafeInteger(item.size)||item.size<1||item.size>binaryLimit||!/^[a-f0-9]{64}$/.test(item.sha256))throw Error('Tool artifact metadata invalid');
  const bytes=file(directory,name,binaryLimit);
  if(bytes.length!==item.size||hash(bytes)!==item.sha256)throw Error('Tool artifact hash differs');
 }
 return {directory:root(directory),manifest,component:target(directory,names.component),probe:target(directory,names.probe)};
}
export function writeMechanismEvidence(directory,commit,tools,seed,receipt){
 identity(commit);
 if(receipt.passed!==true||receipt.commit!==commit||receipt.kind!=='mechanism'||receipt.cleanupErrors?.length)throw Error('Mechanism acceptance is incomplete');
 tools=readTools(tools.directory,commit);
 mkdirSync(directory,{recursive:true});root(directory);
 rmSync(target(directory,'evidence.json'),{force:true});
 const toolsPath=target(directory,'tools');mkdirSync(toolsPath,{recursive:true});
 writeTools(toolsPath,commit,file(tools.directory,names.component,binaryLimit),file(tools.directory,names.probe,binaryLimit),tools.manifest.proof);
 write(directory,'legacy-seed.json',Buffer.from(JSON.stringify(seed)+'\n'));
 write(directory,'mechanism-receipt.json',Buffer.from(JSON.stringify(receipt,null,2)+'\n'));
 const files=['tools/tools.json','legacy-seed.json','mechanism-receipt.json'];
 const manifest={schemaVersion:1,kind:'sayweave-mechanism-evidence',commit,files:Object.fromEntries(files.map(name=>[name,hash(file(directory,name))]))};
 write(directory,'evidence.json',Buffer.from(JSON.stringify(manifest,null,2)+'\n'));
 return readMechanismEvidence(directory,commit);
}
export function readMechanismEvidence(directory,commit){
 identity(commit);const manifest=JSON.parse(file(directory,'evidence.json'));
 const names=['legacy-seed.json','mechanism-receipt.json','tools/tools.json'];
 if(manifest.schemaVersion!==1||manifest.kind!=='sayweave-mechanism-evidence'||manifest.commit!==commit||Object.keys(manifest.files??{}).sort().join()!==names.join())throw Error('Mechanism evidence identity differs');
 for(const name of names){const bytes=file(directory,name);if(hash(bytes)!==manifest.files[name])throw Error('Mechanism evidence hash differs');}
 const tools=readTools(target(directory,'tools'),commit),receipt=JSON.parse(file(directory,'mechanism-receipt.json'));
 if(receipt.passed!==true||receipt.commit!==commit||receipt.kind!=='mechanism'||receipt.cleanupErrors?.length||JSON.stringify(receipt.tools)!==JSON.stringify(tools.manifest))throw Error('Mechanism acceptance is incomplete');
 return {tools,receipt,seed:JSON.parse(file(directory,'legacy-seed.json'))};
}
