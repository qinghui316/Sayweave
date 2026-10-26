import {spawnSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {mkdirSync,readFileSync,writeFileSync} from 'node:fs';
import {fileURLToPath} from 'node:url';
import path from 'node:path';
import {setupLocalAsrTools} from './setup-local-asr-tools.mjs';
import {buildLocalAsr,nativeBuildEnvironment,validateLocalRuntime} from './build-local-asr.mjs';

const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
await setupLocalAsrTools(root);
const runtime=buildLocalAsr(root,{debug:true});
const env={...nativeBuildEnvironment(root),SAYWEAVE_LOCAL_ASR_MANIFEST:runtime.manifestFile};
const commands=[
 ['build','--locked','--manifest-path','src-tauri/local-asr-protocol/Cargo.toml','--features','fixture','--bin','local-asr-fixture'],
 ['test','--locked','--manifest-path','src-tauri/local-asr-protocol/Cargo.toml','--features','fixture'],
 ['test','--locked','--manifest-path','src-tauri/local-asr-worker/Cargo.toml','--lib'],
 ['clippy','--locked','--manifest-path','src-tauri/local-asr-worker/Cargo.toml','--all-targets','--','-D','warnings'],
 ['check','--locked','--manifest-path','src-tauri/Cargo.toml','--features','desktop-acceptance'],
 ['test','--locked','--manifest-path','src-tauri/Cargo.toml','--lib','local_asr::tests','--','--ignored','--nocapture'],
 ['test','--locked','--manifest-path','src-tauri/Cargo.toml','--lib','stalled_adapter_consumer','--','--ignored','--nocapture'],
];
for(const args of commands){
 const split=args.indexOf('--'),shared=['--target-dir',path.resolve(root,env.CARGO_TARGET_DIR??'src-tauri/target')];
 const actual=split<0?[...args,...shared]:[...args.slice(0,split),...shared,...args.slice(split)];
 const result=spawnSync('cargo',actual,{cwd:root,env,stdio:'inherit',windowsHide:true});
 if(result.error)throw result.error;if(result.status!==0)throw Error('Local ASR contract check failed');
}
const hash=data=>createHash('sha256').update(data).digest('hex');
validateLocalRuntime(runtime.directory,runtime.manifest);
mkdirSync(path.join(root,'.local/local-asr'),{recursive:true});
writeFileSync(path.join(root,'.local/local-asr/contracts.json'),JSON.stringify({checkedAt:new Date().toISOString(),source:runtime.manifest.source,manifestSha256:hash(readFileSync(runtime.manifestFile)),worker:runtime.manifest.files['sayweave-local-asr-worker.exe'],commands,passed:true},null,2)+'\n','utf8');
