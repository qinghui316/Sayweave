import {createHash} from 'node:crypto';
import {mkdirSync,readFileSync,writeFileSync,lstatSync,realpathSync,existsSync} from 'node:fs';
import path from 'node:path';

const limit=128*1024*1024;
const hash=bytes=>createHash('sha256').update(bytes).digest('hex');
const names={old:'test-0.1.700-setup.exe',next:'test-0.1.701-setup.exe'};
function item(directory,name,max=limit){
 const root=lstatSync(directory);if(!root.isDirectory()||root.isSymbolicLink())throw Error('Package artifact directory must be real');
 if(!Object.values(names).flatMap(n=>[n,`${n}.sig`]).concat('packages.json').includes(name))throw Error('Unknown package artifact filename');
 const file=path.join(realpathSync(directory),name),info=lstatSync(file);
 if(!info.isFile()||info.isSymbolicLink()||info.size<1||info.size>max)throw Error('Package artifact file exceeds bound or contains a link');
 const bytes=readFileSync(file);if(bytes.length!==info.size)throw Error('Package changed during read');return bytes;
}
function context(manifest,commit){
 if(!/^[a-f0-9]{40}$/.test(commit)||manifest.commit!==commit||manifest.schemaVersion!==1||manifest.kind!=='sayweave-test-packages'||manifest.target!=='windows-x86_64')throw Error('Package artifact commit/identity differs');
 const uuid=manifest.identifier?.slice('local.sayweave.acceptance.'.length);
 if(typeof manifest.identifier!=='string'||!manifest.identifier.startsWith('local.sayweave.acceptance.')||!/^([a-f0-9]{8}-)([a-f0-9]{4}-){3}[a-f0-9]{12}$/.test(uuid)||manifest.product!==`Sayweave 验收 ${uuid}`||!Number.isSafeInteger(manifest.port)||manifest.port<1024||manifest.port>65535||manifest.url!==`https://localhost:${manifest.port}/latest.json`||typeof manifest.publicKey!=='string'||manifest.publicKey.length<32||manifest.publicKey.length>4096||!/^[a-f0-9]{64}$/.test(manifest.componentSha256))throw Error('Package build context differs');
 if(!manifest.artifacts?.old||Object.keys(manifest.artifacts).some(key=>!Object.hasOwn(names,key)))throw Error('Package inventory is incomplete');
}
export function writePackages(directory,commit,build,entries,startupProof){
 mkdirSync(directory,{recursive:true});if(lstatSync(directory).isSymbolicLink())throw Error('Package output contains a link');
 const manifest={schemaVersion:1,kind:'sayweave-test-packages',commit,target:'windows-x86_64',identifier:build.identifier,product:build.product,port:build.port,url:`https://localhost:${build.port}/latest.json`,publicKey:build.publicKey,componentSha256:build.componentSha256,artifacts:{},startupProof};
 for(const [key,entry] of Object.entries(entries)){
  if(!Object.hasOwn(names,key))throw Error('Unknown package');
  const bytes=readFileSync(entry.installer),signature=readFileSync(`${entry.installer}.sig`);if(!bytes.length||bytes.length>limit||!signature.length||signature.length>16384)throw Error('Package output exceeds bound');
  for(const [name,data] of [[names[key],bytes],[`${names[key]}.sig`,signature]]){const file=path.join(directory,name);if(existsSync(file)&&lstatSync(file).isSymbolicLink())throw Error('Package output contains a link');writeFileSync(file,data);}
  manifest.artifacts[key]={file:names[key],size:bytes.length,sha256:hash(bytes),signatureSha256:hash(signature),executableSha256:entry.executableSha256};
 }
 context(manifest,commit);const file=path.join(directory,'packages.json');if(existsSync(file)&&lstatSync(file).isSymbolicLink())throw Error('Package output contains a link');writeFileSync(file,JSON.stringify(manifest,null,2)+'\n');return readPackages(directory,commit);
}
export function readPackages(directory,commit){
 const manifest=JSON.parse(item(directory,'packages.json',1024*1024));context(manifest,commit);
 const entries={};
 for(const [key,entry] of Object.entries(manifest.artifacts)){
  if(entry.file!==names[key]||!Number.isSafeInteger(entry.size)||entry.size<1||entry.size>limit||![entry.sha256,entry.signatureSha256,entry.executableSha256].every(h=>typeof h==='string'&&/^[a-f0-9]{64}$/.test(h)))throw Error('Package artifact metadata invalid');
  const bytes=item(directory,entry.file),signature=item(directory,`${entry.file}.sig`,16384);
  if(bytes.length!==entry.size||hash(bytes)!==entry.sha256||hash(signature)!==entry.signatureSha256)throw Error('Package artifact hash differs');
  entries[key]={installer:path.join(realpathSync(directory),entry.file),executableSha256:entry.executableSha256};
 }
 return {manifest,entries,old:entries.old.installer,next:entries.next?.installer??null};
}
