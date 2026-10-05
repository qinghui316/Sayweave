import {spawnSync} from 'node:child_process';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const mode=process.argv[2];
if(!['dev','build'].includes(mode))throw new Error('Use desktop.mjs dev|build');
function run(file,args,env=process.env){const result=spawnSync(file,args,{cwd:root,env,stdio:'inherit',windowsHide:true});if(result.error)throw result.error;if(result.status!==0)process.exit(result.status??1);}
if(process.platform!=='win32')throw new Error('Windows text-service builds require Windows. Browser dev/build remain portable.');
run('cargo',['build','--manifest-path','src-tauri/text-service/Cargo.toml','--release','--lib']);
const env={...process.env,SAYWEAVE_TSF_PAYLOAD:path.join(root,'src-tauri/text-service/target/release/sayweave_text_service.dll')};
run(process.execPath,[path.join(root,'node_modules/@tauri-apps/cli/tauri.js'),mode,...(mode==='build'?['--no-bundle']:[]),...process.argv.slice(3)],env);
