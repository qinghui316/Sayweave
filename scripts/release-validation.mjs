import {createHash} from 'node:crypto';
import {execFileSync,spawnSync} from 'node:child_process';
import {lstatSync,readFileSync,readdirSync,writeFileSync} from 'node:fs';
import {createRequire} from 'node:module';
import {join,resolve} from 'node:path';
import {pathToFileURL} from 'node:url';
export const repository='qinghui316/Sayweave';
export function releaseIdentity(version,tag,commit){
 if(!/^(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)$/.test(version)||tag!==`v${version}`||!/^[a-f0-9]{40}$/.test(commit))throw new Error('Release identity requires exact version, tag and full commit');
 return {version,tag,commit,installer:`Sayweave_${version}_x64-setup.exe`};
}
export function verifyBuild(identity,root,versions){
 if(versions.some(version=>version!==identity.version))throw new Error('Package, Rust, Tauri and lock versions differ');
 const git=args=>execFileSync('git',args,{cwd:root,encoding:'utf8'}).trim();
 if(git(['rev-parse','HEAD'])!==identity.commit||git(['rev-parse',`${identity.tag}^{commit}`])!==identity.commit)throw new Error('Build HEAD/tag differs from expected commit');
 execFileSync('git',['merge-base','--is-ancestor',identity.commit,'origin/main'],{cwd:root,stdio:'pipe'});
 if(git(['status','--porcelain','--untracked-files=no']))throw new Error('Formal release requires a clean workspace');
}
export function validateRelease(identity,manifest,receipt,assets){
 const installer=identity.installer,signatureName=`${installer}.sig`;
 const expected=[installer,signatureName,'latest.json','release-receipt.json','SHA256SUMS.txt'];
 if(assets.size!==expected.length||expected.some(name=>!assets.has(name)))throw new Error('Release asset inventory differs');
 if(manifest.version!==identity.version||receipt.version!==identity.version||receipt.tag!==identity.tag||receipt.commit!==identity.commit||receipt.installer!==installer||receipt.schemaVersion!==1)throw new Error('Release receipt identity differs');
 const target=manifest.platforms?.['windows-x86_64'];
 const expectedUrl=`https://github.com/${repository}/releases/download/${identity.tag}/${installer}`;
 if(target?.url!==expectedUrl||typeof target.signature!=='string')throw new Error('Installer URL/signature identity differs');
 const bytes=assets.get(installer),hash=createHash('sha512').update(bytes).digest('base64');
 if(bytes.length===0||bytes.length>128*1024*1024||bytes.length!==receipt.installerSize||hash!==receipt.installerSha512||target.installerSize!==bytes.length||target.installerSha512!==hash)throw new Error('Installer hash/size differs');
 if(assets.get(signatureName).toString('utf8').trim()!==target.signature)throw new Error('Manifest signature differs');
 const lines=assets.get('SHA256SUMS.txt').toString('utf8').trim().split(/\r?\n/),names=new Set();
 if(lines.length!==4)throw new Error('Checksum inventory must contain four entries');
 for(const line of lines){
  const match=/^([a-f0-9]{64}) {2}([A-Za-z0-9_.-]+)$/.exec(line);
  if(!match||!expected.slice(0,4).includes(match[2])||names.has(match[2]))throw new Error('Invalid or duplicate checksum identity');
  names.add(match[2]);
  if(createHash('sha256').update(assets.get(match[2])).digest('hex')!==match[1])throw new Error('Checksum differs');
 }
 return target;
}
export function releaseNotes(value){
 if(typeof value.version!=='string'||!Array.isArray(value.changes)||value.changes.some(change=>typeof change!=='string'))throw new Error('Invalid release notes');
 return `Sayweave ${value.version}\n\n${value.changes.map(change=>`- ${change}`).join('\n')}\n${value.installation?`\n${value.installation}\n`:''}`;
}

export const frozenTestPaths=['tests/e2e/models.spec.ts','tests/e2e/ui-interaction.spec.ts'];
const controllerPaths=new Set([...frozenTestPaths,'.github/workflows/windows-release.yml','scripts/release-validation.mjs','scripts/verify-update-release.mjs','tests/release-validation.test.ts','docs/release.md']);
export function validateFrozenController(tag,product,controller,ref,paths){
 if(!/^v\d+\.\d+\.\d+$/.test(tag)||! /^[a-f0-9]{40}$/.test(product)||! /^[a-f0-9]{40}$/.test(controller)||product===controller)throw Error('Exact distinct frozen product/controller identities required');
 if(!ref.startsWith(`refs/tags/${tag}-release-check-`)||! /^[a-z0-9][a-z0-9.-]*$/.test(ref.slice(`refs/tags/${tag}-release-check-`.length)))throw Error('Reviewed diagnostic controller tag required');
 if(paths.length===0||paths.some(path=>!controllerPaths.has(path)))throw Error('Controller delta includes product or undeclared files');
}

export async function runFrozenReleaseTests(root=process.cwd(),env=process.env,run){
 const tag=env.SAYWEAVE_RELEASE_TAG,product=env.SAYWEAVE_RELEASE_COMMIT,controller=env.SAYWEAVE_CONTROLLER_COMMIT,ref=env.SAYWEAVE_CONTROLLER_REF;
 const git=(args,encoding='utf8')=>execFileSync('git',args,{cwd:root,encoding});
 const paths=git(['diff','--name-only',product,controller]).trim().split(/\r?\n/).filter(Boolean);
 validateFrozenController(tag,product,controller,ref,paths);
 if(git(['rev-parse',`${ref}^{commit}`]).trim()!==controller)throw Error('Controller tag identity differs');
 for(const commit of [product,controller])git(['merge-base','--is-ancestor',commit,'origin/main']);
 git(['merge-base','--is-ancestor',product,controller]);
 const original=createRequire(import.meta.url)(join(root,'scripts/release-validation.mjs'));
 const version=JSON.parse(readFileSync(join(root,'package.json'),'utf8')).version;
 original.verifyBuild(original.releaseIdentity(version,tag,product),root,[version]);
 const testSources=frozenTestPaths.map(path=>{const bytes=git(['show',`${controller}:${path}`],null);new TextDecoder('utf-8',{fatal:true}).decode(bytes);return {path,bytes,sha256:createHash('sha256').update(bytes).digest('hex')};});
 console.log(JSON.stringify({productTag:tag,productCommit:product,controllerCommit:controller,controllerRef:ref,testSources:testSources.map(({path,sha256})=>({path,sha256}))}));
 let failure;
 try{
  for(const {path,bytes} of testSources)writeFileSync(join(root,path),bytes);
  const result=run?await run():spawnSync(process.execPath,[join(root,'node_modules/@playwright/test/cli.js'),'test'],{cwd:root,env,stdio:'inherit',windowsHide:true});
  if(result?.error||result?.status!==0)throw Error('Reviewed frozen-product E2E failed',{cause:result?.error});
 }catch(error){failure=error;}
 try{
  git(['restore','--source',product,'--worktree','--',...frozenTestPaths]);
  if(git(['status','--porcelain','--untracked-files=no']).trim())throw Error('Frozen product checkout was not restored cleanly');
 }catch(error){throw new AggregateError(failure?[failure,error]:[error],'Frozen product restoration failed',{cause:error});}
 if(failure)throw failure;
}
export function validateOwnedDraft(identity,release,id){
 if(!/^[1-9]\d*$/.test(id)||!Number.isSafeInteger(release?.id)||String(release.id)!==id||release.tag_name!==identity.tag||release.target_commitish!==identity.commit||release.draft!==true||release.prerelease!==false||!Array.isArray(release.assets)||release.assets.length!==0)throw Error('Exact empty owner draft required; existing assets are protected');
}
export function validateVerificationRun(identity,run,jobs,id){
 if(!/^[1-9]\d*$/.test(id)||String(run?.id)!==id||run.repository?.full_name!==repository||run.path!=='.github/workflows/windows-release.yml'||run.event!=='workflow_dispatch'||run.status!=='completed'||! /^[a-f0-9]{40}$/.test(run.head_sha)||!run.head_branch?.startsWith(`${identity.tag}-release-check-`))throw Error('Verification run identity differs');
 const selected=jobs.filter(job=>job.name==='verify');
 const required=['Verify frozen product and landed controller','Run npm run check','Run reviewed E2E against the frozen product and restore its checkout','Run npm run check:distribution'];
 if(selected.length!==1||selected[0].status!=='completed'||selected[0].conclusion!=='success'||required.some(name=>selected[0].steps.filter(step=>step.name===name&&step.conclusion==='success').length!==1))throw Error('Prior product verification is incomplete');
}
export function verifyReusedChecks(root=process.cwd(),env=process.env){
 const id=env.SAYWEAVE_VERIFICATION_RUN;if(!/^[1-9]\d*$/.test(id))throw Error('Exact verification run required');
 const api=endpoint=>JSON.parse(execFileSync('gh',['api',`repos/${repository}/${endpoint}`],{cwd:root,env,encoding:'utf8'}));
 const git=(args,encoding='utf8')=>execFileSync('git',args,{cwd:root,encoding});
 const identity=releaseIdentity(JSON.parse(readFileSync(join(root,'package.json'),'utf8')).version,env.SAYWEAVE_RELEASE_TAG,env.SAYWEAVE_RELEASE_COMMIT);
 const original=createRequire(import.meta.url)(join(root,'scripts/release-validation.mjs'));original.verifyBuild(identity,root,[identity.version]);
 const run=api(`actions/runs/${id}`),jobs=api(`actions/runs/${id}/jobs?per_page=100`).jobs;
 validateVerificationRun(identity,run,jobs,id);
 for(const [commit,ref] of [[run.head_sha,`refs/tags/${run.head_branch}`],[env.SAYWEAVE_CONTROLLER_COMMIT,env.SAYWEAVE_CONTROLLER_REF]]){
  validateFrozenController(identity.tag,identity.commit,commit,ref,git(['diff','--name-only',identity.commit,commit]).trim().split(/\r?\n/).filter(Boolean));
  if(git(['rev-parse',`${ref}^{commit}`]).trim()!==commit)throw Error('Verification controller tag differs');
  git(['merge-base','--is-ancestor',commit,'origin/main']);git(['merge-base','--is-ancestor',identity.commit,commit]);
 }
 for(const path of frozenTestPaths)if(!git(['show',`${run.head_sha}:${path}`],null).equals(git(['show',`${env.SAYWEAVE_CONTROLLER_COMMIT}:${path}`],null)))throw Error('Reviewed E2E source changed; fresh checks required');
 console.log(JSON.stringify({reusedVerificationRun:id,verifyJob:jobs.find(job=>job.name==='verify').id,productCommit:identity.commit,sourceController:run.head_sha,controllerCommit:env.SAYWEAVE_CONTROLLER_COMMIT,scope:'full check, reviewed full E2E and distribution; identical frozen product/tests'}));
}

const sha256=bytes=>createHash('sha256').update(bytes).digest('hex');
const exactId=id=>/^[1-9]\d*$/.test(String(id));
export const verifierArtifactName=identity=>`release-verifier-${identity.tag}-${identity.commit}`;
export const packageArtifactName=identity=>`release-package-${identity.tag}-${identity.commit}`;
export function cargoCacheKeys(context){
 // Explicit public inputs only: never serialize the environment or Cargo home.
 const {platform,arch,toolchain,cargo,image,config,locks,source}=context;
 if(platform!=='win32'||arch!=='x64'||![toolchain,cargo,image,config,locks,source].every(v=>typeof v==='string'&&v.length>0))throw Error('Incomplete Windows Cargo cache context');
 const abi=sha256(JSON.stringify({platform,arch,toolchain,cargo,image,config,target:'x86_64-pc-windows-msvc',profiles:['debug','release']}));
 const prefix=`cargo-windows-v1-${abi}-`;
 return {compilePrefix:prefix,compileKey:`${prefix}${sha256(locks)}-${sha256(source)}`,registryPrefix:'cargo-public-crates-v1-windows-',registryKey:`cargo-public-crates-v1-windows-${sha256(locks)}`};
}
export function nativeSourceHash(root){
 const entries=execFileSync('git',['ls-tree','-r','--full-tree','HEAD'],{cwd:root,encoding:'utf8'}).trim().split(/\r?\n/).filter(line=>{const p=line.split('\t')[1];return p?.startsWith('src-tauri/')||p==='rust-toolchain.toml'||p==='package-lock.json';});
 if(!entries.some(e=>e.endsWith('\tsrc-tauri/src/bin/verify_update_signature.rs')))throw Error('Full native signature verifier source missing');
 return sha256(entries.sort().join('\n'));
}
export function cargoCacheContext(root=process.cwd(),env=process.env){
 const git=args=>execFileSync('git',args,{cwd:root,encoding:'utf8'}).trim();
 const configPaths=git(['ls-files']).split(/\r?\n/).filter(p=>/^(?:\.cargo\/|src-tauri\/(?:text-service\/)?\.cargo\/)|(?:^|\/)(?:Cargo\.toml|rust-toolchain\.toml|tauri\.[^/]+\.json)$/.test(p));
 const config={files:configPaths.map(p=>[p,sha256(readFileSync(join(root,p)))]),flags:Object.fromEntries(Object.entries(env).filter(([key])=>/^(?:RUSTFLAGS|CARGO_ENCODED_RUSTFLAGS|CARGO_BUILD_TARGET|CARGO_PROFILE_\w+|CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_\w+)$/.test(key)).sort(([a],[b])=>a.localeCompare(b)))};
 const context={platform:process.platform,arch:process.arch,toolchain:execFileSync('rustc',['-vV'],{cwd:root,env,encoding:'utf8'}).trim(),cargo:execFileSync('cargo',['-V'],{cwd:root,env,encoding:'utf8'}).trim(),image:`${env.ImageOS??'local'}:${env.ImageVersion??'local'}`,config:JSON.stringify(config),locks:JSON.stringify(['src-tauri/Cargo.lock','src-tauri/text-service/Cargo.lock'].map(p=>sha256(readFileSync(join(root,p))))),source:`${nativeSourceHash(root)}:${env.SAYWEAVE_BUILD_CHANNEL??'internal'}`};
 return {...cargoCacheKeys(context),context};
}
export function validateProducerRun(identity,run,jobs,id,current=false){
 if(!exactId(id)||String(run?.id)!==String(id)||run.repository?.full_name!==repository||run.path!=='.github/workflows/windows-release.yml'||!['push','workflow_dispatch'].includes(run.event)||! /^[a-f0-9]{40}$/.test(run.head_sha)||!(run.head_branch===identity.tag||run.head_branch?.startsWith(`${identity.tag}-release-check-`)))throw Error('Package/tool producer identity differs');
 if(!current){
  const draft=jobs.filter(j=>j.name==='draft');
  const required=['Build signed NSIS package','Build full signature verifier once','Preserve full signature verifier','Preserve signed release bytes before Release operations'];
  if(run.status!=='completed'||draft.length!==1||required.some(name=>draft[0].steps.filter(s=>s.name===name&&s.conclusion==='success').length!==1))throw Error('Package/tool producer did not preserve a complete build');
 }
 return {runId:String(id),controllerCommit:run.head_sha,ref:`refs/tags/${run.head_branch}`,repository};
}
export function trustedProducer(identity,root,env=process.env){
 const id=env.SAYWEAVE_PACKAGE_RUN||env.GITHUB_RUN_ID;
 if(!exactId(id))throw Error('Exact package/tool producer run required');
 const current=String(id)===env.GITHUB_RUN_ID;
 const api=endpoint=>JSON.parse(execFileSync('gh',['api',`repos/${repository}/${endpoint}`],{cwd:root,env,encoding:'utf8'}));
 const run=api(`actions/runs/${id}`),jobs=current?[]:api(`actions/runs/${id}/jobs?per_page=100`).jobs;
 const producer=validateProducerRun(identity,run,jobs,id,current);
 const git=args=>execFileSync('git',args,{cwd:root,encoding:'utf8'}).trim();
 if(current&&producer.controllerCommit!==env.SAYWEAVE_CONTROLLER_COMMIT)throw Error('Current workflow source differs');
 if(git(['rev-parse',`${producer.ref}^{commit}`])!==producer.controllerCommit)throw Error('Producer controller tag differs');
 git(['merge-base','--is-ancestor',producer.controllerCommit,'origin/main']);
 if(producer.controllerCommit!==identity.commit)validateFrozenController(identity.tag,identity.commit,producer.controllerCommit,producer.ref,git(['diff','--name-only',identity.commit,producer.controllerCommit]).split(/\r?\n/).filter(Boolean));
 else if(producer.ref!==`refs/tags/${identity.tag}`)throw Error('Product producer tag differs');
 return producer;
}
export function verifierBuildConfig(root,env){
 const git=args=>execFileSync('git',args,{cwd:root,encoding:'utf8'}).trim();
 const files=git(['ls-files']).split(/\r?\n/).filter(p=>/^(?:\.cargo\/|src-tauri\/\.cargo\/)/.test(p));
 return sha256(JSON.stringify({files:files.map(p=>[p,sha256(readFileSync(join(root,p)))]),flags:Object.fromEntries(Object.entries(env).filter(([key])=>/^(?:RUSTFLAGS|CARGO_ENCODED_RUSTFLAGS|CARGO_BUILD_TARGET|CARGO_PROFILE_\w+|CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_\w+)$/.test(key)).sort(([a],[b])=>a.localeCompare(b)))}));
}
export function validateVerifierTool(identity,sourceHash,toolchain,buildConfig,receipt,bytes,producer){
 if(receipt?.schemaVersion!==1||receipt.productCommit!==identity.commit||receipt.productTag!==identity.tag||receipt.sourceSha256!==sourceHash||receipt.buildConfigSha256!==buildConfig||receipt.file!=='verify-update-signature.exe'||receipt.platform!=='windows-x86_64-msvc'||receipt.profile!=='release'||receipt.features!=='default'||receipt.buildChannel!=='stable'||receipt.toolchain!==toolchain||!receipt.rustc?.startsWith(`rustc ${toolchain} `)||!receipt.rustc.includes('host: x86_64-pc-windows-msvc')||receipt.size!==bytes.length||bytes.length===0||receipt.sha256!==sha256(bytes)||JSON.stringify(receipt.producer)!==JSON.stringify(producer))throw Error('Native verifier source, build identity or bytes differ');
}
export function readPlainFile(path){
 if(lstatSync(path).isSymbolicLink()||!lstatSync(path).isFile())throw Error('Artifact file must be a regular file');
 return readFileSync(path);
}
export function releaseAssets(dir){
 if(lstatSync(dir).isSymbolicLink()||!lstatSync(dir).isDirectory())throw Error('Artifact directory must be a plain directory');
 const names=readdirSync(dir);
 if(names.some(n=>! /^[A-Za-z0-9_.-]+$/.test(n)))throw Error('Invalid asset name');
 return new Map(names.map(n=>[n,readPlainFile(join(dir,n))]));
}
export function validateRetainedArtifacts(identity,artifacts,id){
 for(const name of [packageArtifactName(identity),verifierArtifactName(identity)]){
  const found=artifacts.filter(a=>a.name===name);
  if(found.length!==1||found[0].expired!==false||!Number.isSafeInteger(found[0].id)||String(found[0].workflow_run?.id)!==String(id))throw Error('Exact retained package/tool artifacts are unavailable');
 }
}
export function validateUploadInventory(identity,release,id,assets){
 validateDraftIdentity(identity,release,id);
 const seen=new Set();
 for(const asset of release.assets){
  const bytes=assets.get(asset.name);
  if(!bytes||seen.has(asset.name)||asset.state!=='uploaded'||asset.size!==bytes.length||asset.digest!==`sha256:${sha256(bytes)}`)throw Error('Existing draft asset differs; never overwrite');
  seen.add(asset.name);
 }
 return [...assets.keys()].filter(n=>!seen.has(n));
}
export function validateDraftIdentity(identity,release,id){
 if(!exactId(id)||String(release?.id)!==String(id)||release.tag_name!==identity.tag||release.target_commitish!==identity.commit||release.draft!==true||release.prerelease!==false||!Array.isArray(release.assets))throw Error('Protected draft identity differs');
}
function releaseApi(root,env,endpoint){return JSON.parse(execFileSync('gh',['api',`repos/${repository}/${endpoint}`],{cwd:root,env,encoding:'utf8'}));}
function currentIdentity(root,env){return releaseIdentity(JSON.parse(readFileSync(join(root,'package.json'),'utf8')).version,env.SAYWEAVE_RELEASE_TAG,env.SAYWEAVE_RELEASE_COMMIT);}
function output(value,env){console.log(JSON.stringify(value));if(env.GITHUB_OUTPUT)for(const [key,v] of Object.entries(value))writeFileSync(env.GITHUB_OUTPUT,`${key}=${typeof v==='string'?v:JSON.stringify(v)}\n`,{flag:'a'});}
export function preflightPackage(root=process.cwd(),env=process.env){
 const identity=currentIdentity(root,env),original=createRequire(import.meta.url)(join(root,'scripts/release-validation.mjs'));original.verifyBuild(identity,root,[identity.version]);
 const releases=env.EXPECTED_RELEASE_ID?[releaseApi(root,env,`releases/${env.EXPECTED_RELEASE_ID}`)]:releaseApi(root,env,'releases?per_page=100').filter(r=>r.tag_name===identity.tag);
 if(releases.length!==1)throw Error('Owner must first create one exact draft with publication permissions');
 const release=releases[0],id=String(release.id);
 if(env.SAYWEAVE_RELEASE_PHASE==='resume-package'){
  const producer=trustedProducer(identity,root,env);
  validateRetainedArtifacts(identity,releaseApi(root,env,`actions/runs/${producer.runId}/artifacts?per_page=100`).artifacts,producer.runId);
  // Exact asset bytes/signatures are checked after artifact download, before any write.
  validateDraftIdentity(identity,release,id);
 }else{
  validateOwnedDraft(identity,release,id);
  for(let page=1;;page++){
   const list=releaseApi(root,env,`actions/artifacts?per_page=100&page=${page}`).artifacts;
   if(list.some(a=>a.name===packageArtifactName(identity)&&a.expired===false&&String(a.workflow_run?.id)!==env.GITHUB_RUN_ID))throw Error('A retained product package already exists; resume-package is required');
   if(list.length<100)break;
  }
 }
 output({release_id:id,package_run:env.SAYWEAVE_PACKAGE_RUN||env.GITHUB_RUN_ID},env);
}
export function uploadRetainedPackage(root=process.cwd(),env=process.env){
 const identity=currentIdentity(root,env),dir=resolve(root,env.SAYWEAVE_RELEASE_DIR??'release/sayweave'),assets=releaseAssets(dir);
 validateRelease(identity,JSON.parse(assets.get('latest.json')),JSON.parse(assets.get('release-receipt.json')),assets);
 if(env.EXPECTED_CHECKSUM&&sha256(assets.get('SHA256SUMS.txt'))!==env.EXPECTED_CHECKSUM)throw Error('Retained checksum differs');
 const id=env.EXPECTED_RELEASE_ID;
 // Artifact retention is a hard predecessor, including a tool usable by later jobs.
 const producer=trustedProducer(identity,root,env);
 validateRetainedArtifacts(identity,releaseApi(root,env,`actions/runs/${producer.runId}/artifacts?per_page=100`).artifacts,producer.runId);
 let release=releaseApi(root,env,`releases/${id}`);
 const missing=validateUploadInventory(identity,release,id,assets);
 for(const name of missing){
  // Recheck ID and every existing asset at each boundary; no clobber or replacement.
  release=releaseApi(root,env,`releases/${id}`);validateUploadInventory(identity,release,id,assets);
  execFileSync('gh',['release','upload',identity.tag,'--repo',repository,join(dir,name)],{cwd:root,env,stdio:'inherit',windowsHide:true});
 }
 release=releaseApi(root,env,`releases/${id}`);
 if(validateUploadInventory(identity,release,id,assets).length)throw Error('Draft upload is incomplete');
 output({release_id:String(id),checksum:sha256(assets.get('SHA256SUMS.txt')),package_run:producer.runId},env);
}
if(process.argv[1]&&pathToFileURL(resolve(process.argv[1])).href===import.meta.url){
 const mode=process.argv[2];
 if(mode==='draft-check')validateOwnedDraft(releaseIdentity(JSON.parse(readFileSync('package.json','utf8')).version,process.env.SAYWEAVE_RELEASE_TAG,process.env.SAYWEAVE_RELEASE_COMMIT),JSON.parse(readFileSync(process.argv[3],'utf8').replace(/^\uFEFF/,'')),process.env.EXPECTED_RELEASE_ID);
 else if(mode==='reuse-verification')verifyReusedChecks();
 else if(mode==='cargo-cache'){const {compileKey,compilePrefix,registryKey,registryPrefix}=cargoCacheContext();output({compileKey,compilePrefix,registryKey,registryPrefix},process.env);}
 else if(mode==='package-preflight')preflightPackage();
 else if(mode==='upload-package')uploadRetainedPackage();
 else if(mode===undefined)runFrozenReleaseTests().catch(error=>{console.error(error);process.exitCode=1;});
 else throw Error('Unknown frozen release validation mode');
}
