import {createHash} from 'node:crypto';
import {execFileSync,spawnSync} from 'node:child_process';
import {readFileSync,writeFileSync} from 'node:fs';
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
const controllerPaths=new Set([...frozenTestPaths,'.github/workflows/windows-release.yml','scripts/release-validation.mjs','tests/release-validation.test.ts','docs/release.md']);
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
if(process.argv[1]&&pathToFileURL(resolve(process.argv[1])).href===import.meta.url){
 const mode=process.argv[2];
 if(mode==='draft-check')validateOwnedDraft(releaseIdentity(JSON.parse(readFileSync('package.json','utf8')).version,process.env.SAYWEAVE_RELEASE_TAG,process.env.SAYWEAVE_RELEASE_COMMIT),JSON.parse(readFileSync(process.argv[3],'utf8').replace(/^\uFEFF/,'')),process.env.EXPECTED_RELEASE_ID);
 else if(mode==='reuse-verification')verifyReusedChecks();
 else if(mode===undefined)runFrozenReleaseTests().catch(error=>{console.error(error);process.exitCode=1;});
 else throw Error('Unknown frozen release validation mode');
}
