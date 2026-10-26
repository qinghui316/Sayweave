import {spawnSync} from 'node:child_process';
import path from 'node:path';
import { buildTextService,verifyComponentEmbeddings } from './build-text-service.mjs';
import {buildLocalAsr} from './build-local-asr.mjs';
import {setupLocalAsrTools} from './setup-local-asr-tools.mjs';
import {fileURLToPath} from 'node:url';
import {readFileSync,writeFileSync,mkdirSync,existsSync} from 'node:fs';
import {createHash} from 'node:crypto';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const mode=process.argv[2];
if(!['dev','build','package'].includes(mode))throw new Error('Use desktop.mjs dev|build|package');
function run(file,args,env=process.env){const result=spawnSync(file,args,{cwd:root,env,stdio:'inherit',windowsHide:true});if(result.error)throw result.error;if(result.status!==0)process.exit(result.status??1);}
if(process.platform!=='win32')throw new Error('Windows text-service builds require Windows. Browser dev/build remain portable.');
// A release may reuse its independently verified, unchanged component artifact.
// The caller must bind its source revision/hash; the default builds from source.
const components=buildTextService(root);
await setupLocalAsrTools(root);
const requestedTarget=process.argv.indexOf('--target');
const local=buildLocalAsr(root,{debug:mode==='dev'||process.argv.includes('--debug'),...(requestedTarget>=0?{target:process.argv[requestedTarget+1]}:{})});
const env={...process.env,SAYWEAVE_TSF_PAYLOAD:components.core,SAYWEAVE_TSF_PROXY_PAYLOAD:components.proxy,SAYWEAVE_LOCAL_ASR_MANIFEST:local.manifestFile};
let extra=process.argv.slice(3);
let packagedHelper;
if(mode==='package'){
 const pinned=JSON.parse(readFileSync(path.join(root,'src-tauri/windows/installer-source.json'),'utf8'));
 const cli=JSON.parse(readFileSync(path.join(root,'node_modules/@tauri-apps/cli/package.json'),'utf8'));
 const binding=readFileSync(path.join(root,'node_modules/@tauri-apps/cli-win32-x64-msvc/cli.win32-x64-msvc.node'));
 const digest=data=>createHash('sha256').update(data).digest('hex');
 if(cli.version!==pinned.cliVersion||digest(binding)!==pinned.bindingSha256||digest(binding.subarray(pinned.templateOffset,pinned.templateOffset+pinned.templateLength))!==pinned.templateSha256)throw Error('Pinned NSIS template source no longer matches the installed CLI.');
 let config={},rest=[];
 for(let i=0;i<extra.length;i++){
  if(extra[i]==='--config'){
   const value=extra[++i];if(!value)throw Error('Missing packaging configuration');
   const incoming=JSON.parse(value.trim().startsWith('{')?value:readFileSync(path.resolve(root,value),'utf8'));
   config={...config,...incoming,bundle:{...config.bundle,...incoming.bundle,windows:{...config.bundle?.windows,...incoming.bundle?.windows,nsis:{...config.bundle?.windows?.nsis,...incoming.bundle?.windows?.nsis}}}};
  }else rest.push(extra[i]);
 }
 const base=JSON.parse(readFileSync(path.join(root,'src-tauri/tauri.conf.json'),'utf8'));
 for(const value of [base,config]){
  if(value.bundle?.resources||value.bundle?.externalBin||value.bundle?.fileAssociations||value.bundle?.deepLinkProtocols)throw Error('Installer backup manifest must be extended before adding owned resources or associations.');
  if(value.bundle?.publisher&&value.bundle.publisher!=='sayweave')throw Error('Installer manufacturer namespace must stay sayweave.');
  if(value.bundle?.windows?.nsis?.startMenuFolder)throw Error('Installer only supports the fixed root start-menu shortcut.');
  if(value.bundle?.windows?.nsis?.installMode&&value.bundle.windows.nsis.installMode!=='currentUser')throw Error('Installer transaction requires currentUser mode.');
  if(value.bundle?.windows?.nsis?.template&&value.bundle.windows.nsis.template!=='windows/installer.nsi')throw Error('Installer transaction requires the pinned custom template.');
  if(value.bundle?.windows?.nsis?.installerHooks)throw Error('Installer hooks must be incorporated into the transaction contract first.');
 }
 const features=rest.indexOf('--features'),target=rest.indexOf('--target'),debug=rest.includes('--debug');
 const helperEnv={...env,TAURI_CONFIG:JSON.stringify(config)};
 // This package's library also compiles its GUI context. Produce the same
 // standard frontend before compiling the helper in a fresh source archive.
 run(process.execPath,[path.join(root,'node_modules/typescript/bin/tsc'),'--noEmit'],env);
 run(process.execPath,[path.join(root,'node_modules/vite/bin/vite.js'),'build'],env);
 run('cargo',['build','--manifest-path','src-tauri/Cargo.toml','--bin','sayweave-installer-helper',...(debug?[]:['--release']),...(features>=0?['--features',rest[features+1]]:[]),...(target>=0?['--target',rest[target+1]]:[])],helperEnv);
 const output=path.resolve(root,env.CARGO_TARGET_DIR??'src-tauri/target',...(target>=0?[rest[target+1]]:[]),debug?'debug':'release','sayweave-installer-helper.exe');
 const bytes=readFileSync(output),sha=createHash('sha256').update(bytes).digest('hex');
 console.log(JSON.stringify({componentPayloads:verifyComponentEmbeddings([output],components)}));
 const directory=path.join(root,'.local','installer-helper',sha);mkdirSync(directory,{recursive:true});
 const helper=path.join(directory,'sayweave-installer-helper.exe');
 packagedHelper=helper;
 if(existsSync(helper)){if(!readFileSync(helper).equals(bytes))throw Error('Installer helper cache identity differs.');}else writeFileSync(helper,bytes,{flag:'wx'});
 const localResources=Object.fromEntries(Object.keys(local.manifest.files).map(name=>[path.join(local.directory,name),`local-asr/${name}`]));
 const combined={...config,build:{...config.build,beforeBuildCommand:''},bundle:{...config.bundle,resources:{[helper]:'sayweave-installer-helper.exe',...localResources}}};
 const configFile=path.join(directory,'package-config.json');writeFileSync(configFile,JSON.stringify(combined));
 extra=[...rest,'--config',configFile];
}
const tauriMode=mode==='package'?'build':mode;
run(process.execPath,[path.join(root,'node_modules/@tauri-apps/cli/tauri.js'),tauriMode,...(mode==='build'?['--no-bundle']:mode==='package'?['--bundles','nsis']:[]),...extra],env);
if(mode!=='dev'){
 const target=extra.indexOf('--target');
 const executable=path.resolve(root,env.CARGO_TARGET_DIR??'src-tauri/target',...(target>=0?[extra[target+1]]:[]),extra.includes('--debug')?'debug':'release','sayweave.exe');
 const receipt=verifyComponentEmbeddings([executable,...(packagedHelper?[packagedHelper]:[])],components);
 console.log(JSON.stringify({componentPayloads:receipt}));
 mkdirSync(path.join(root,'.local/component-build'),{recursive:true});
 writeFileSync(path.join(root,'.local/component-build/payload-receipt.json'),JSON.stringify(receipt,null,2)+'\n');
}
