import {createHash} from 'node:crypto';
import {execFileSync,spawnSync} from 'node:child_process';
import {copyFileSync,existsSync,mkdirSync,readFileSync,writeFileSync} from 'node:fs';
import {join,resolve} from 'node:path';
import {pathToFileURL} from 'node:url';
import {releaseIdentity,verifyBuild,validateRelease,releaseAssets,readPlainFile,nativeSourceHash,verifierBuildConfig,validateVerifierTool,trustedProducer} from './release-validation.mjs';

function buildIdentity(root,env){
 const pkg=JSON.parse(readFileSync(join(root,'package.json'),'utf8'));
 const identity=releaseIdentity(pkg.version,env.SAYWEAVE_RELEASE_TAG,env.SAYWEAVE_RELEASE_COMMIT);
 const config=JSON.parse(readFileSync(join(root,'src-tauri/tauri.conf.json'),'utf8')),lock=JSON.parse(readFileSync(join(root,'package-lock.json'),'utf8'));
 const rust=readFileSync(join(root,'src-tauri/Cargo.toml'),'utf8').match(/^version = "([^"]+)"/m)?.[1];
 const rustLock=readFileSync(join(root,'src-tauri/Cargo.lock'),'utf8').match(/\[\[package\]\]\r?\nname = "sayweave"\r?\nversion = "([^"]+)"/)?.[1];
 verifyBuild(identity,root,[config.version,lock.version,lock.packages[''].version,rust,rustLock]);
 const toolchain=readFileSync(join(root,'rust-toolchain.toml'),'utf8').match(/^channel = "([^"]+)"/m)?.[1];
 if(!toolchain||env.SAYWEAVE_BUILD_CHANNEL!=='stable')throw Error('Pinned stable verifier build context required');
 return {identity,config,toolchain};
}
function producer(identity,root,env){
 if(env.GITHUB_ACTIONS==='true')return trustedProducer(identity,root,env);
 return {runId:'local',controllerCommit:identity.commit,ref:`refs/tags/${identity.tag}`,repository:'qinghui316/Sayweave'};
}
export function buildVerifier(root=process.cwd(),env=process.env){
 const {identity,toolchain}=buildIdentity(root,env);
 if(process.platform!=='win32'||process.arch!=='x64')throw Error('Native verifier requires Windows x64');
 const dir=resolve(root,env.SAYWEAVE_VERIFIER_DIR??'release/verifier');
 if(existsSync(join(dir,'verifier-receipt.json')))throw Error('A recorded verifier already exists; reuse it instead of rebuilding');
 const payload=resolve(root,'src-tauri/text-service/target/release/sayweave_text_service.dll');
 if(!existsSync(payload))throw Error('Build the ordinary component/package before the verifier');
 const sourceProducer=producer(identity,root,env);
 const rustc=execFileSync(env.RUSTC??env.CARGO_BUILD_RUSTC??'rustc',['-vV'],{cwd:root,env,encoding:'utf8'}).trim();
 if(!rustc.startsWith(`rustc ${toolchain} `)||!rustc.includes('host: x86_64-pc-windows-msvc'))throw Error('Actual verifier compiler differs from the pinned Windows toolchain');
 const buildEnv={...env,SAYWEAVE_BUILD_COMMIT:identity.commit,SAYWEAVE_TSF_PAYLOAD:payload};
 execFileSync('cargo',['build','--locked','--release','--manifest-path','src-tauri/Cargo.toml','--bin','verify-update-signature'],{cwd:root,env:buildEnv,stdio:'inherit',windowsHide:true});
 const file='verify-update-signature.exe',bytes=readPlainFile(join(root,'src-tauri/target/release',file));
 const receipt={schemaVersion:1,productTag:identity.tag,productCommit:identity.commit,sourceSha256:nativeSourceHash(root),buildConfigSha256:verifierBuildConfig(root,env),file,platform:'windows-x86_64-msvc',profile:'release',features:'default',buildChannel:'stable',toolchain,rustc,size:bytes.length,sha256:createHash('sha256').update(bytes).digest('hex'),producer:sourceProducer};
 mkdirSync(dir,{recursive:true});copyFileSync(join(root,'src-tauri/target/release',file),join(dir,file));
 writeFileSync(join(dir,'verifier-receipt.json'),JSON.stringify(receipt,null,2)+'\n',{flag:'wx'});
 console.log(JSON.stringify({verifierBuilds:1,...receipt}));
}
export function checkVerifier(root=process.cwd(),env=process.env){
 const {identity,toolchain}=buildIdentity(root,env);
 const toolDir=resolve(root,env.SAYWEAVE_VERIFIER_DIR??'release/verifier');
 if(!existsSync(join(toolDir,'verifier-receipt.json')))throw Error('Recorded native verifier is required; build-tool once, then reuse its verified CI artifact');
 const toolFiles=releaseAssets(toolDir);
 if(toolFiles.size!==2||!toolFiles.has('verify-update-signature.exe')||!toolFiles.has('verifier-receipt.json'))throw Error('Native verifier artifact inventory differs');
 const toolReceipt=JSON.parse(toolFiles.get('verifier-receipt.json'));
 validateVerifierTool(identity,nativeSourceHash(root),toolchain,verifierBuildConfig(root,env),toolReceipt,toolFiles.get('verify-update-signature.exe'),producer(identity,root,env));
 return {toolDir,toolReceipt};
}
export function verifyUpdateRelease(root=process.cwd(),env=process.env){
 const {identity,config}=buildIdentity(root,env),dir=resolve(root,env.SAYWEAVE_RELEASE_DIR??'release/sayweave');
 const assets=releaseAssets(dir),manifest=JSON.parse(assets.get('latest.json')),receipt=JSON.parse(assets.get('release-receipt.json'));
 validateRelease(identity,manifest,receipt,assets);
 const {toolDir,toolReceipt}=checkVerifier(root,env);
 const publicKey=config.plugins?.updater?.pubkey;if(typeof publicKey!=='string'||!publicKey)throw Error('Updater public key missing');
 // The same complete Rust/minisign implementation as the client, without Cargo.
 const verification=spawnSync(join(toolDir,'verify-update-signature.exe'),[publicKey,join(dir,identity.installer),join(dir,`${identity.installer}.sig`),identity.version],{cwd:root,encoding:'utf8',stdio:['ignore','pipe','pipe'],windowsHide:true});
 if(verification.error||verification.status!==0)throw Error(`Updater signature/version verification failed: ${verification.stderr||verification.stdout}`,{cause:verification.error});
 console.log(JSON.stringify({verifiedRelease:identity.tag,productCommit:identity.commit,verifierSha256:toolReceipt.sha256,verifierProducer:toolReceipt.producer,verification:'full native minisign and signed version; no Cargo invocation'}));
}
if(process.argv[1]&&pathToFileURL(resolve(process.argv[1])).href===import.meta.url){
 if(process.argv[2]==='build-tool')buildVerifier();
 else if(process.argv[2]==='check-tool')console.log(JSON.stringify(checkVerifier().toolReceipt));
 else if(process.argv[2]===undefined)verifyUpdateRelease();
 else throw Error('Use verify-update-release.mjs [build-tool|check-tool]');
}
