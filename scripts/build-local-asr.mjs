import {createHash,randomUUID} from 'node:crypto';
import {spawnSync} from 'node:child_process';
import {copyFileSync,existsSync,lstatSync,mkdirSync,readFileSync,readdirSync,renameSync,unlinkSync,writeFileSync} from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';

const sha=bytes=>createHash('sha256').update(bytes).digest('hex');
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
// Cargo/build scripts rewrite their output files in place. A runtime payload
// must own separate bytes; hardlinks to those files are not immutable snapshots.
export function stageRuntimeFile(from,to,identity){
 if(existsSync(to)){
  const stat=lstatSync(to);
  if(!stat.isFile()||stat.isSymbolicLink())throw Error('Local runtime destination type differs');
  if(stat.nlink===1&&stat.size===identity.bytes&&sha(readFileSync(to))===identity.sha256)return;
 }
 const temporary=to+'.'+randomUUID()+'.stage';
 try{
  copyFileSync(from,temporary);
  const bytes=readFileSync(temporary);
  if(bytes.length!==identity.bytes||sha(bytes)!==identity.sha256)throw Error('Local runtime producer changed during staging');
  renameSync(temporary,to);
 }finally{if(existsSync(temporary))unlinkSync(temporary);}
}
export function runtimeNames(project=root){return JSON.parse(readFileSync(path.join(project,'src-tauri/local-asr-protocol/runtime-files.json'),'utf8'));}
function sourceIdentity(project,debug,target){
 const files={};
 for(const owner of ['local-asr-worker','local-asr-protocol']){
  function walk(dir){for(const name of readdirSync(dir)){const file=path.join(dir,name),stat=lstatSync(file);if(stat.isSymbolicLink())throw Error('Local runtime source link');if(stat.isDirectory()){if(name!=='target')walk(file);}else if(/\.(rs|toml|lock|json)$/u.test(name))files[path.relative(project,file).replaceAll('\\','/')]=sha(readFileSync(file));}}
  walk(path.join(project,'src-tauri',owner));
 }
 return sha(JSON.stringify({files,toolchain:sha(readFileSync(path.join(project,'rust-toolchain.toml'))),debug,target}));
}
export function validateLocalRuntime(directory,manifest,project=root){
 const names=runtimeNames(project).sort();
 if(manifest.version!==1||manifest.engine!=='0.1.3'||manifest.target!=='x86_64-pc-windows-msvc'||JSON.stringify(Object.keys(manifest.files??{}).sort())!==JSON.stringify(names))throw Error('Local runtime inventory differs');
 for(const name of names){
  const file=path.join(directory,name),stat=lstatSync(file),entry=manifest.files[name];
  if(!stat.isFile()||stat.isSymbolicLink()||stat.nlink!==1||stat.size<128||stat.size>128*1024**2||entry.bytes!==stat.size)throw Error('Local runtime file bound/type differs');
  const bytes=readFileSync(file);
  if(sha(bytes)!==entry.sha256)throw Error('Local runtime hash differs');
  if(name==='licenses.txt'){
   if(bytes.length>128*1024||new TextDecoder('utf-8',{fatal:true}).decode(bytes).includes('\0'))throw Error('Local runtime notices differ');
  }else{
   const pe=bytes.readUInt32LE(0x3c);
   if(!bytes.subarray(0,2).equals(Buffer.from('MZ'))||pe<64||pe+6>bytes.length||bytes.readUInt32LE(pe)!==0x4550||bytes.readUInt16LE(pe+4)!==0x8664)throw Error('Local runtime binary differs');
  }
 }
 for(const name of readdirSync(directory))if(/\.(dll|exe)$/iu.test(name)&&!names.includes(name))throw Error('Unexpected local runtime binary');
 return manifest;
}
function run(program,args,env,cwd){const result=spawnSync(program,args,{cwd,env,stdio:'inherit',windowsHide:true});if(result.error)throw result.error;if(result.status!==0)throw Error(`Local runtime build failed (${result.status})`);}
export function nativeBuildEnvironment(project=root){
 const env={...process.env,CL:[process.env.CL??'','/utf-8'].join(' ').trim()};
 const vswhere=path.join(process.env['ProgramFiles(x86)']??'C:/Program Files (x86)','Microsoft Visual Studio/Installer/vswhere.exe');
 const found=spawnSync(vswhere,['-latest','-products','*','-property','installationPath'],{encoding:'utf8',windowsHide:true});
 if(found.status===0){const cmake=path.join(found.stdout.trim(),'Common7/IDE/CommonExtensions/Microsoft/CMake/CMake/bin');const key=Object.keys(env).find(key=>key.toLowerCase()==='path')??'PATH';env[key]=cmake+path.delimiter+(env[key]??'');}
 if(!env.VULKAN_SDK){const minimal=path.join(project,'.local/tools/vulkan-minimal');if(existsSync(path.join(minimal,'bin/glslc.exe')))env.VULKAN_SDK=minimal;}
 if(!env.VULKAN_SDK)throw Error('Install a Vulkan development SDK or run setup-local-asr-tools.mjs. End users do not need the SDK.');
 return env;
}
export function buildLocalAsr(project=root,{debug=false,target,stagingDirectory}={}){
 if(process.platform!=='win32'||target&&target!=='x86_64-pc-windows-msvc')throw Error('Local runtime v1 requires Windows x64 MSVC');
 const output=path.resolve(project,process.env.CARGO_TARGET_DIR??'src-tauri/target',...(target?[target]:[]),debug?'debug':'release');
 const directory=stagingDirectory?path.resolve(stagingDirectory):path.join(output,'local-asr'),manifestFile=path.join(directory,'manifest.json'),source=sourceIdentity(project,debug,target??'host-x86_64-pc-windows-msvc');
 const notices=readFileSync(path.join(project,'src-tauri/local-asr-worker/THIRD_PARTY_NOTICES.txt'));
 if(existsSync(manifestFile)){
  const saved=JSON.parse(readFileSync(manifestFile,'utf8'));
  if(saved.source===source){try{
   if(saved.files?.['licenses.txt']?.sha256!==sha(notices)){
    const to=path.join(directory,'licenses.txt');
    if(existsSync(to)&&lstatSync(to).isSymbolicLink())throw Error('Local runtime notices link');
    const identity={bytes:notices.length,sha256:sha(notices)};
    stageRuntimeFile(path.join(project,'src-tauri/local-asr-worker/THIRD_PARTY_NOTICES.txt'),to,identity);saved.files['licenses.txt']=identity;
   }
   validateLocalRuntime(directory,saved,project);writeFileSync(manifestFile,JSON.stringify(saved,null,2)+'\n','utf8');return {directory,manifestFile,manifest:saved,reused:true};
  }catch{/* rebuild exact invalidated payload */}}
 }
 const env=nativeBuildEnvironment(project);
 run('cargo',['build','--locked','--manifest-path','src-tauri/local-asr-worker/Cargo.toml','--bin','sayweave-local-asr-worker','--target-dir',path.resolve(project,process.env.CARGO_TARGET_DIR??'src-tauri/target'),...(target?['--target',target]:[]),...(debug?[]:['--release'])],env,project);
 const manifest=JSON.parse(readFileSync(path.join(output,'local-asr-runtime-manifest.json'),'utf8'));
 const worker='sayweave-local-asr-worker.exe',bytes=readFileSync(path.join(output,worker));
 manifest.files[worker]={bytes:bytes.length,sha256:sha(bytes)};manifest.source=source;
 manifest.files['licenses.txt']={bytes:notices.length,sha256:sha(notices)};
 mkdirSync(directory,{recursive:true});
 for(const name of Object.keys(manifest.files)){
  const from=name==='licenses.txt'?path.join(project,'src-tauri/local-asr-worker/THIRD_PARTY_NOTICES.txt'):path.join(output,name),to=path.join(directory,name);
  stageRuntimeFile(from,to,manifest.files[name]);
 }
 validateLocalRuntime(directory,manifest,project);writeFileSync(manifestFile,JSON.stringify(manifest,null,2)+'\n','utf8');
 return {directory,manifestFile,manifest,reused:false};
}
if(process.argv[1]&&path.resolve(process.argv[1])===fileURLToPath(import.meta.url)){
 const result=buildLocalAsr(root,{debug:process.argv.includes('--debug')});
 console.log(JSON.stringify({directory:result.directory,manifest:result.manifestFile,reused:result.reused,worker:result.manifest.files['sayweave-local-asr-worker.exe']}));
}
