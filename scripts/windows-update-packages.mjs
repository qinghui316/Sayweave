import {createHash} from 'node:crypto';
import {mkdirSync,readFileSync,writeFileSync,lstatSync,realpathSync,existsSync} from 'node:fs';
import path from 'node:path';
import {retainInstaller} from './storage-policy.mjs';

const limit=128*1024*1024;
const hash=bytes=>createHash('sha256').update(bytes).digest('hex');
export function nsisPayloadInventory(listing){
 if(typeof listing!=='string'||listing.length>2*1024*1024||!/^Type = Nsis\r?$/m.test(listing))throw Error('NSIS inventory format differs');
 const normalized=listing.replaceAll('\r\n','\n'),marker='\n----------\n',start=normalized.indexOf(marker);
 if(start<0)throw Error('NSIS inventory records missing');
 const paths=[...normalized.slice(start+marker.length).matchAll(/^Path = ([^\r\n]+)$/gm)].map(match=>match[1]);
 if(paths.length<6||paths.length>64||paths.some(value=>value.length>2048||value.includes('\0')||value.split(/[\\/]/).some(part=>part==='..'||part==='.'||part==='')))throw Error('NSIS inventory path exceeds contract');
 const installed=paths.filter(value=>!value.startsWith('$PLUGINSDIR\\')).sort();
 if(JSON.stringify(installed)!==JSON.stringify(['sayweave-installer-helper.exe','sayweave.exe','uninstall.exe']))throw Error('NSIS installed payload expands or duplicates the owned file manifest');
 for(const name of ['sayweave-main.exe','sayweave-maintenance.exe','sayweave-uninstall.exe'])if(paths.filter(value=>value===`$PLUGINSDIR\\${name}`).length!==1)throw Error('NSIS transaction payload missing or duplicated');
 return {installed,temporaryCount:paths.length-installed.length};
}
export function nsisHelperIdentity(installed,maintenance,frozen=installed){
 if(!Buffer.isBuffer(installed)||!Buffer.isBuffer(maintenance)||!Buffer.isBuffer(frozen)||installed.length<2||installed.length>limit||!installed.subarray(0,2).equals(Buffer.from('MZ'))||!installed.equals(maintenance)||!installed.equals(frozen))throw Error('NSIS installed, transaction and frozen helpers differ');
 return {helperSha256:hash(installed),helperSize:installed.length};
}
// Tauri 2.12.1 patches the NSIS payload token, then restores the build output.
// Bind the generated NSIS input, actual signed payload and restored source bytes.
export function nsisExecutableIdentity(script,expectedSource,buildOutput,payload){
 const matches=[...script.matchAll(/^!define MAINBINARYSRCPATH "([^"\r\n]+)"\r?$/gm)];
 if(matches.length!==1)throw Error('NSIS main binary source is missing or ambiguous');
 const source=matches[0][1].replaceAll('$$','$');
 if(!path.win32.isAbsolute(source)||path.win32.normalize(source).toLowerCase()!==path.win32.normalize(expectedSource).toLowerCase())throw Error('NSIS main binary source differs from the frozen build output');
 if(!Buffer.isBuffer(buildOutput)||!Buffer.isBuffer(payload)||buildOutput.length<1||payload.length!==buildOutput.length||payload.length>limit)throw Error('NSIS executable bytes exceed identity bounds');
 const restored=Buffer.from(payload),token=Buffer.from('__TAURI_BUNDLE_TYPE_VAR_NSS'),original=Buffer.from('__TAURI_BUNDLE_TYPE_VAR_UNK');
 const offset=restored.indexOf(token);
 if(offset<0||restored.indexOf(token,offset+1)>=0||buildOutput.indexOf(original)!==offset||buildOutput.indexOf(original,offset+1)>=0)throw Error('NSIS bundle type identity is missing or ambiguous');
 original.copy(restored,offset);
 if(!restored.equals(buildOutput))throw Error('NSIS payload differs beyond the expected bundle type patch');
 return {mainBinarySource:source,restoredBuildSha256:hash(buildOutput),executableSha256:hash(payload),executableSize:payload.length,bundleTypeOffset:offset};
}
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
  if(directory===path.join(process.cwd(),'.local','windows-update-packages'))manifest.artifacts[key].storage=retainInstaller(process.cwd(),path.join(directory,names[key]),{productCommit:commit,purpose:'immutable acceptance '+key,expectedSha256:hash(bytes)});
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
