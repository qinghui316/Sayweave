import {createHash} from 'node:crypto';
import {execFileSync} from 'node:child_process';
import {mkdirSync,mkdtempSync,readFileSync,rmSync,writeFileSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join,resolve,sep} from 'node:path';
import {describe,expect,it} from 'vitest';
// The public release tools are JavaScript entry points with separately exercised inputs.
// @ts-expect-error JavaScript release validator has no declaration file.
import {releaseIdentity,validateRelease,releaseNotes,validateFrozenController,runFrozenReleaseTests} from '../scripts/release-validation.mjs';
function fixture(){
 const identity=releaseIdentity('0.1.7','v0.1.7','a'.repeat(40)),bytes=Buffer.from('synthetic installer'),signature='synthetic signature';
 const sha512=createHash('sha512').update(bytes).digest('base64');
 const target={signature,url:'https://github.com/qinghui316/Sayweave/releases/download/v0.1.7/Sayweave_0.1.7_x64-setup.exe',installerSize:bytes.length,installerSha512:sha512};
 const manifest={version:'0.1.7',platforms:{'windows-x86_64':target}};
 const receipt={schemaVersion:1,...identity,installerSize:bytes.length,installerSha512:sha512};
 const assets=new Map<string,Buffer>([[identity.installer,bytes],[`${identity.installer}.sig`,Buffer.from(signature)],['latest.json',Buffer.from(JSON.stringify(manifest))],['release-receipt.json',Buffer.from(JSON.stringify(receipt))]]);
 const sums=()=>[...assets].filter(([name])=>name!=='SHA256SUMS.txt').map(([name,bytes])=>`${createHash('sha256').update(bytes).digest('hex')}  ${name}`).join('\n');
 assets.set('SHA256SUMS.txt',Buffer.from(sums()));return{identity,manifest,receipt,assets,sums};
}
describe('public release identity and inventory',()=>{
 it('accepts an exact inventory and generates readable notes',()=>{const f=fixture();expect(validateRelease(f.identity,f.manifest,f.receipt,f.assets)).toBe(f.manifest.platforms['windows-x86_64']);expect(releaseNotes({version:'0.1.7',changes:['修复更新']})).toContain('- 修复更新');});
 it.each(['unknown','b'.repeat(40),'a'.repeat(39)])('rejects a false receipt commit %s',commit=>{const f=fixture();f.receipt.commit=commit;expect(()=>validateRelease(f.identity,f.manifest,f.receipt,f.assets)).toThrow();});
 it.each(['https://evil.example/releases/download/v0.1.7/Sayweave_0.1.7_x64-setup.exe','https://github.com/qinghui316/Sayweave/releases/download/v0.1.7/wrong.exe'])('rejects an ambiguous URL %s',url=>{const f=fixture();f.manifest.platforms['windows-x86_64'].url=url;expect(()=>validateRelease(f.identity,f.manifest,f.receipt,f.assets)).toThrow();});
 it.each(['missing','duplicate','traversal','extra'])('rejects %s checksum entries',kind=>{const f=fixture(),lines=f.sums().split('\n');if(kind==='missing')lines.pop();if(kind==='duplicate')lines[3]=lines[0]!;if(kind==='traversal')lines[0]=lines[0]!.replace(f.identity.installer,'../bad.exe');if(kind==='extra')lines.push(lines[0]!);f.assets.set('SHA256SUMS.txt',Buffer.from(lines.join('\n')));expect(()=>validateRelease(f.identity,f.manifest,f.receipt,f.assets)).toThrow();});
 it('rejects extra assets and a version without an exact commit',()=>{const f=fixture();f.assets.set('extra.exe',Buffer.from('extra'));expect(()=>validateRelease(f.identity,f.manifest,f.receipt,f.assets)).toThrow();expect(()=>releaseIdentity('0.1.7','v0.1.7','unknown')).toThrow();});
});

describe('frozen product release controller boundaries',()=>{
 const product='a'.repeat(40),controller='b'.repeat(40),ref='refs/tags/v0.1.9-release-check-e2e-1';
 it.each(['src/app/App.tsx','src-tauri/src/lib.rs','package.json','.github/workflows/windows-ci.yml','../tests/e2e/models.spec.ts'])('rejects undeclared/product change %s',path=>{
  expect(()=>validateFrozenController('v0.1.9',product,controller,ref,['tests/e2e/models.spec.ts',path])).toThrow();
 });
 it('requires distinct exact identities and the matching diagnostic controller ref',()=>{
  for(const bad of ['refs/heads/main','refs/tags/v0.1.8-release-check-e2e-1','refs/tags/v0.1.9-release-check-'])expect(()=>validateFrozenController('v0.1.9',product,controller,bad,['tests/e2e/models.spec.ts'])).toThrow();
  expect(()=>validateFrozenController('v0.1.9',product,product,ref,['tests/e2e/models.spec.ts'])).toThrow();
  expect(()=>validateFrozenController('v0.1.9',product,'unknown',ref,['tests/e2e/models.spec.ts'])).toThrow();
 });
 it.each([0,1,2])('preserves the frozen checkout and failure facts for outcome %s',async status=>{
  const root=mkdtempSync(join(tmpdir(),'beaver-frozen-release-'));
  const owned=resolve(root);if(!owned.startsWith(resolve(tmpdir())+sep)||!owned.split(sep).at(-1)?.startsWith('beaver-frozen-release-'))throw Error('Unexpected temporary fixture path');
  const git=(args:string[])=>execFileSync('git',args,{cwd:root,encoding:'utf8',stdio:['ignore','pipe','pipe']}).trim();
  try{
   mkdirSync(join(root,'scripts'),{recursive:true});mkdirSync(join(root,'tests/e2e'),{recursive:true});
   writeFileSync(join(root,'package.json'),JSON.stringify({version:'0.1.9'}));
   writeFileSync(join(root,'scripts/release-validation.mjs'),readFileSync('scripts/release-validation.mjs'));
   for(const path of ['models','ui-interaction'])writeFileSync(join(root,`tests/e2e/${path}.spec.ts`),'original');
   git(['init']);git(['config','user.name','fixture']);git(['config','user.email','fixture@example.invalid']);git(['config','commit.gpgsign','false']);git(['config','core.autocrlf','false']);git(['config','core.hooksPath',join(root,'no-hooks')]);
   git(['add','.']);git(['commit','-m','product']);const product=git(['rev-parse','HEAD']);git(['tag','v0.1.9']);
   for(const path of ['models','ui-interaction'])writeFileSync(join(root,`tests/e2e/${path}.spec.ts`),'reviewed');
   git(['add','.']);git(['commit','-m','controller']);const controller=git(['rev-parse','HEAD']);git(['tag','v0.1.9-release-check-e2e-1']);git(['update-ref','refs/remotes/origin/main',controller]);git(['checkout','--detach',product]);
   const work=runFrozenReleaseTests(root,{SAYWEAVE_RELEASE_TAG:'v0.1.9',SAYWEAVE_RELEASE_COMMIT:product,SAYWEAVE_CONTROLLER_COMMIT:controller,SAYWEAVE_CONTROLLER_REF:ref},()=>{expect(readFileSync(join(root,'tests/e2e/models.spec.ts'),'utf8')).toBe('reviewed');if(status===2)writeFileSync(join(root,'package.json'),JSON.stringify({version:'0.1.9',unexpected:true}));return {status:status===0?0:1};});
   if(status===0)await work;else if(status===1)await expect(work).rejects.toThrow('E2E failed');else{await expect(work).rejects.toMatchObject({name:'AggregateError',errors:[expect.objectContaining({message:'Reviewed frozen-product E2E failed'}),expect.objectContaining({message:'Frozen product checkout was not restored cleanly'})]});expect(git(['status','--porcelain'])).toContain('package.json');git(['restore','--worktree','--','package.json']);}
   for(const path of ['models','ui-interaction'])expect(readFileSync(join(root,`tests/e2e/${path}.spec.ts`),'utf8')).toBe('original');
   expect(git(['status','--porcelain'])).toBe('');expect(git(['rev-parse','HEAD'])).toBe(product);
   if(status===0){git(['update-ref','refs/remotes/origin/main',product]);let invoked=false;await expect(runFrozenReleaseTests(root,{SAYWEAVE_RELEASE_TAG:'v0.1.9',SAYWEAVE_RELEASE_COMMIT:product,SAYWEAVE_CONTROLLER_COMMIT:controller,SAYWEAVE_CONTROLLER_REF:ref},()=>{invoked=true;return {status:0};})).rejects.toThrow();expect(invoked).toBe(false);expect(git(['status','--porcelain'])).toBe('');}
  }finally{
   rmSync(owned,{recursive:true});
  }
 });
});
