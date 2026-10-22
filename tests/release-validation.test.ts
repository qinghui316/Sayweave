import {createHash} from 'node:crypto';
import {execFileSync} from 'node:child_process';
import {mkdirSync,mkdtempSync,readFileSync,rmSync,writeFileSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join,resolve,sep} from 'node:path';
import {describe,expect,it} from 'vitest';
// The public release tools are JavaScript entry points with separately exercised inputs.
// @ts-expect-error JavaScript release validator has no declaration file.
import {releaseIdentity,validateRelease,releaseNotes,validateFrozenController,validateNativeTestOverride,validateMeterTestOverride,runFrozenReleaseTests,validateOwnedDraft,validateVerificationRun,cargoCacheKeys,validateVerifierTool,validateProducerRun,validateRetainedArtifacts,validateUploadInventory,packageArtifactName,verifierArtifactName,assertNoRetainedPackage} from '../scripts/release-validation.mjs';
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

describe('Cargo reuse identity',()=>{
 const context={platform:'win32',arch:'x64',toolchain:'rustc 1.99.0 msvc',cargo:'cargo 1.99.0',image:'windows2025:20261011',config:'default profiles and flags',locks:'main and component locks',source:'native source and channel'};
 it('keeps controller revisions and credentials outside cache identity',()=>{
  expect(cargoCacheKeys({...context,controllerCommit:'a'.repeat(40),TAURI_SIGNING_PRIVATE_KEY:'private'})).toEqual(cargoCacheKeys({...context,controllerCommit:'b'.repeat(40),TAURI_SIGNING_PRIVATE_KEY:'another'}));
 });
 it.each(['toolchain','cargo','image','config'])('isolates incompatible %s compilation caches',field=>{
  expect(cargoCacheKeys({...context,[field]:'changed'}).compilePrefix).not.toBe(cargoCacheKeys(context).compilePrefix);
 });
 it.each(['locks','source'])('changes the exact %s key but preserves compatible dependencies',field=>{
  const next=cargoCacheKeys({...context,[field]:'changed'}),old=cargoCacheKeys(context);
  expect(next.compileKey).not.toBe(old.compileKey);expect(next.compilePrefix).toBe(old.compilePrefix);
  expect(next.registryKey===old.registryKey).toBe(field==='source');
 });
 it('refuses non-Windows or incomplete contexts',()=>{
  for(const change of [{platform:'linux'},{arch:'arm64'},{toolchain:''},{locks:''}])expect(()=>cargoCacheKeys({...context,...change})).toThrow();
 });
});

describe('recorded full native verifier',()=>{
 const identity=releaseIdentity('0.1.9','v0.1.9','a'.repeat(40)),bytes=Buffer.from('tiny synthetic verifier fixture');
 const producer={runId:'99',controllerCommit:'b'.repeat(40),ref:'refs/tags/v0.1.9-release-check-five',repository:'qinghui316/Sayweave'};
 const receipt=()=>({schemaVersion:1,productTag:identity.tag,productCommit:identity.commit,sourceSha256:'c'.repeat(64),buildConfigSha256:'d'.repeat(64),file:'verify-update-signature.exe',platform:'windows-x86_64-msvc',profile:'release',features:'default',buildChannel:'stable',toolchain:'1.99.0',rustc:'rustc 1.99.0 (fixture)\nhost: x86_64-pc-windows-msvc',size:bytes.length,sha256:createHash('sha256').update(bytes).digest('hex'),producer});
 const validate=(value:unknown,data=bytes)=>validateVerifierTool(identity,'c'.repeat(64),'1.99.0','d'.repeat(64),value,data,producer);
 it('accepts exact source, configuration, producer and tool bytes',()=>expect(()=>validate(receipt())).not.toThrow());
 it.each(['commit','source','config','file','platform','profile','features','channel','toolchain','rustc','size','hash','producer'])('refuses changed %s before executable invocation',field=>{
  const value=receipt();
  if(field==='commit')value.productCommit='f'.repeat(40);if(field==='source')value.sourceSha256='f'.repeat(64);if(field==='config')value.buildConfigSha256='f'.repeat(64);if(field==='file')value.file='../other.exe';if(field==='platform')value.platform='linux';if(field==='profile')value.profile='debug';if(field==='features')value.features='acceptance';if(field==='channel')value.buildChannel='test';if(field==='toolchain')value.toolchain='1.98.0';if(field==='rustc')value.rustc='rustc 1.99.0\nhost: x86_64-pc-windows-gnu';if(field==='size')value.size++;if(field==='hash')value.sha256='f'.repeat(64);if(field==='producer')value.producer={...producer,runId:'100'};
  expect(()=>validate(value)).toThrow();
 });
 it('refuses corrupt bytes even with an unchanged receipt',()=>expect(()=>validate(receipt(),Buffer.from('corrupt'))).toThrow());
});

describe('retained package resume and partial upload',()=>{
 const f=fixture(),producerId='99';
 const run=()=>({id:99,repository:{full_name:'qinghui316/Sayweave'},path:'.github/workflows/windows-release.yml',event:'workflow_dispatch',head_sha:'b'.repeat(40),head_branch:`${f.identity.tag}-release-check-five`,status:'completed',conclusion:'failure'});
 const jobs=()=>[{name:'draft',steps:['Build signed NSIS package','Build full signature verifier once','Preserve full signature verifier','Preserve signed release bytes before Release operations','Verify retained package with recorded full verifier'].map(name=>({name,conclusion:'success'}))}];
 it('allows a later upload failure to reuse a complete preserved build',()=>expect(validateProducerRun(f.identity,run(),jobs(),producerId).runId).toBe(producerId));
 it('reuses a preserved tool without treating it as a preserved package',()=>{
  const toolJobs=[{name:'draft',steps:jobs()[0]!.steps.filter(s=>s.name==='Build full signature verifier once'||s.name==='Preserve full signature verifier')}];
  expect(validateProducerRun(f.identity,run(),toolJobs,producerId,false,'tool').runId).toBe(producerId);
  expect(()=>validateProducerRun(f.identity,run(),toolJobs,producerId)).toThrow();
  toolJobs[0]!.steps[1]!.conclusion='failure';
  expect(()=>validateProducerRun(f.identity,run(),toolJobs,producerId,false,'tool')).toThrow();
 });
 it('validates a later package producer separately from its earlier tool producer',()=>{
  const packageJobs=[{name:'draft',steps:jobs()[0]!.steps.filter(s=>s.name==='Build signed NSIS package'||s.name==='Preserve signed release bytes before Release operations')}];
  expect(validateProducerRun(f.identity,run(),packageJobs,producerId,false,'package').runId).toBe(producerId);
  expect(()=>validateProducerRun(f.identity,run(),packageJobs,producerId,false,'tool')).toThrow();
  expect(()=>validateProducerRun(f.identity,run(),packageJobs,producerId)).toThrow();
  packageJobs[0]!.steps[1]!.conclusion='failure';expect(()=>validateProducerRun(f.identity,run(),packageJobs,producerId,false,'package')).toThrow();
 });
 it('keeps original producer proof when a later rerun was rejected before build',()=>{
  const j=jobs();j.push({name:'draft',steps:[]});expect(validateProducerRun(f.identity,run(),j,producerId).runId).toBe(producerId);
 });
 it('can resume preserved bytes when the first full-verifier execution failed',()=>{
  const j=jobs();j[0]!.steps[4]!.conclusion='failure';expect(validateProducerRun(f.identity,run(),j,producerId).runId).toBe(producerId);
 });
 it('reuses a tool from an earlier attempt without another tool build',()=>{
  const all=jobs()[0]!.steps;
  const attempts=[{name:'draft',steps:all.filter(s=>s.name==='Build full signature verifier once'||s.name==='Preserve full signature verifier')},{name:'draft',steps:all.filter(s=>s.name!=='Build full signature verifier once'&&s.name!=='Preserve full signature verifier')}];
  expect(validateProducerRun(f.identity,run(),attempts,producerId).runId).toBe(producerId);
 });
 it.each(['repo','controller','tag','unfinished','build','tool','preserve','duplicate'])('refuses %s producer',kind=>{
  const r=run(),j=jobs();if(kind==='repo')r.repository.full_name='another/repo';if(kind==='controller')r.head_sha='unknown';if(kind==='tag')r.head_branch='v0.1.8';if(kind==='unfinished')r.status='in_progress';if(kind==='build')j[0]!.steps[0]!.conclusion='failure';if(kind==='tool')j[0]!.steps[1]!.conclusion='skipped';if(kind==='preserve')j[0]!.steps[3]!.conclusion='failure';if(kind==='duplicate')j.push(j[0]!);expect(()=>validateProducerRun(f.identity,r,j,producerId)).toThrow();
 });
 const artifacts=()=>[packageArtifactName(f.identity),verifierArtifactName(f.identity)].map((name,index)=>({name,id:index+1,expired:false,workflow_run:{id:99}}));
 it('rejects rebuild even when the preserved package belongs to this same run',()=>expect(()=>assertNoRetainedPackage(f.identity,artifacts())).toThrow());
 it('does not turn artifact expiry into automatic repackaging',()=>{const a=artifacts();a[0]!.expired=true;expect(()=>assertNoRetainedPackage(f.identity,a)).toThrow();});
 it('requires both exact retained artifacts',()=>expect(()=>validateRetainedArtifacts(f.identity,artifacts(),producerId)).not.toThrow());
 it('keeps separately retained tool and package artifact identities distinct',()=>{
  const a=artifacts();expect(()=>validateRetainedArtifacts(f.identity,[a[0]],producerId,'package')).not.toThrow();expect(()=>validateRetainedArtifacts(f.identity,[a[1]],producerId,'tool')).not.toThrow();
  expect(()=>validateRetainedArtifacts(f.identity,[a[1]],producerId,'package')).toThrow();expect(()=>validateRetainedArtifacts(f.identity,[a[0]],producerId,'tool')).toThrow();
  expect(()=>validateRetainedArtifacts(f.identity,[a[1]],'100','tool')).toThrow();
 });
 it.each(['missing','expired','foreign','duplicate'])('refuses %s retained data',kind=>{
  const a=artifacts();if(kind==='missing')a.pop();if(kind==='expired')a[0]!.expired=true;if(kind==='foreign')a[0]!.workflow_run.id=100;if(kind==='duplicate')a.push(a[0]!);expect(()=>validateRetainedArtifacts(f.identity,a,producerId)).toThrow();
 });
 const draft=()=>({id:42,tag_name:f.identity.tag,target_commitish:f.identity.commit,draft:true,prerelease:false,assets:[...f.assets].slice(0,2).map(([name,bytes])=>({name,size:bytes.length,digest:`sha256:${createHash('sha256').update(bytes).digest('hex')}`,state:'uploaded'}))});
 it('resumes only missing exact assets, retaining already uploaded bytes',()=>expect(validateUploadInventory(f.identity,draft(),'42',f.assets)).toEqual(['latest.json','release-receipt.json','SHA256SUMS.txt']));
 it.each(['hash','size','unknown','duplicate','unfinished','published','commit'])('never overwrites a %s draft asset',kind=>{
  const d=draft();if(kind==='hash')d.assets[0]!.digest='sha256:'+'f'.repeat(64);if(kind==='size')d.assets[0]!.size++;if(kind==='unknown')d.assets[0]!.name='foreign.exe';if(kind==='duplicate')d.assets.push(d.assets[0]!);if(kind==='unfinished')d.assets[0]!.state='new';if(kind==='published')d.draft=false;if(kind==='commit')d.target_commitish='f'.repeat(40);expect(()=>validateUploadInventory(f.identity,d,'42',f.assets)).toThrow();
 });
});

describe('frozen product release controller boundaries',()=>{
 // Reconstruct the frozen fixture without requiring release tags in shallow CI clones.
 // The validator pins each original/reviewed native test suffix through EOF.
 const nativeReviewed=readFileSync('src-tauri/src/installer/record.rs','utf8').replaceAll('\r\n','\n');
 const start=nativeReviewed.indexOf('    fn ordinary_owner_receipt_preserves_owner_and_confirms_replacement() {');
 const end=nativeReviewed.indexOf('    #[test]\n    fn live_installer_identity_fences_receipt_recovery_including_pid_reuse()',start);
 const nativeOriginal=nativeReviewed.slice(0,start)+`    fn ordinary_owner_receipt_preserves_owner_and_confirms_replacement() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("receipt.json");
        write_receipt(&path, b"old").unwrap();
        write_receipt(&path, b"new").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"new");
        sayweave_text_service::atomic_record::verify_user_owner(&path).unwrap();
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }
`+nativeReviewed.slice(end);
 const startupExpectedBefore="            let expected = format!(\n                \"\\\"{}\\\" --autostart\",\n                fixture.0.join(\"sayweave.exe\").display()\n            );\n            assert_eq!(value, expected);\n            assert!(owns(&value, &canonical));\n";
 const startupExpectedAfter="            let executable = fixture.0.join(\"sayweave.exe\").canonicalize().unwrap();\n            let executable = executable.to_str().unwrap().strip_prefix(r\"\\\\?\\\").unwrap();\n            assert_eq!(executable.as_bytes().get(1..3), Some(b\":\\\\\".as_slice()));\n            let expected = format!(\"\\\"{executable}\\\" --autostart\");\n            assert_eq!(value, expected);\n            assert!(owns(&value, &canonical));\n            let path = metadata::wide(fixture.0.to_str().unwrap());\n            let mut short = vec![0u16; 32768];\n            let length = unsafe {\n                windows::Win32::Storage::FileSystem::GetShortPathNameW(\n                    windows::core::PCWSTR(path.as_ptr()),\n                    Some(&mut short),\n                )\n            } as usize;\n            assert!(length > 0 && length < short.len());\n            let short = std::path::PathBuf::from(String::from_utf16(&short[..length]).unwrap());\n            let alias = metadata::normalized_directory(&short).unwrap();\n            assert_eq!(alias, canonical);\n            assert_eq!(command(&alias).unwrap(), value);\n            println!(\"owned_startup_fixture_short_alias={}\", short != fixture.0);\n";
 const continuationFixtures=[
  {path:'src-tauri/src/installer/login_startup.rs',before:`            let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .join(".local")
                .join(format!("启动路径 test {}", uuid::Uuid::new_v4()));`,after:`            let root = tempfile::tempdir().unwrap();
            let directory = root.path().join("启动路径 test");`},
  {path:'src-tauri/src/data/archive_recovery.rs',before:`        assert_eq!(entries(temp.path()).unwrap().len(), 2);
        drop(repo);
        let repo = Repository::open(temp.path()).unwrap();`,after:`        let pending = entries(temp.path()).unwrap().len();
        assert_eq!(pending, 2);
        drop(repo);
        let mut repo = Repository::open(temp.path()).unwrap();
        // Startup replay is bounded. Continue the same maintenance path without
        // requiring both receipts to fit in its first 25ms pass on every disk.
        for _ in 0..pending {
            replay_batch(&mut repo, temp.path(), false, std::time::Duration::ZERO, 1).unwrap();
        }`},
 ].map(({path,before,after})=>{const reviewed=readFileSync(path,'utf8').replaceAll('\r\n','\n');let original=reviewed.replace(after,before);if(path==='src-tauri/src/installer/login_startup.rs')original=original.replace(startupExpectedAfter,startupExpectedBefore);return {path,reviewed,original};});
 const meterReviewed=readFileSync('tests/voice-edit-resume.test.tsx','utf8').replaceAll('\r\n','\n');
 const meterFixture={path:'tests/voice-edit-resume.test.tsx',reviewed:meterReviewed,original:meterReviewed.replace(" vi.spyOn(performance,'now').mockReturnValue(1000);\n",'').replace('vi.restoreAllMocks();','')};
 const frozenFixtures=[...continuationFixtures,meterFixture];
 it('pins the complete original and reviewed meter clock test',()=>{
  expect(meterFixture.original).not.toBe(meterFixture.reviewed);
  expect(()=>validateMeterTestOverride(Buffer.from(meterFixture.original),Buffer.from(meterFixture.reviewed))).not.toThrow();
 });
 it.each(['append','clock','original','swap'])('rejects unreviewed standalone meter fixture: %s',kind=>{
  let original=meterFixture.original,reviewed=meterFixture.reviewed;
  if(kind==='append')reviewed+='\nexport const unreviewed=true;\n';
  if(kind==='clock')reviewed=reviewed.replace('mockReturnValue(1000)','mockReturnValue(1001)');
  if(kind==='original')original+='\n';
  if(kind==='swap')[original,reviewed]=[reviewed,original];
  expect(()=>validateMeterTestOverride(Buffer.from(original),Buffer.from(reviewed))).toThrow();
 });
 it.each(continuationFixtures)('permits the exact continuation fixture for $path',({path,original,reviewed})=>{
  expect(original).not.toBe(reviewed);
  expect(()=>validateNativeTestOverride(Buffer.from(original),Buffer.from(reviewed),path)).not.toThrow();
 });
 it.each(continuationFixtures.flatMap(fixture=>['production','append','original','test','path'].map(kind=>({...fixture,kind}))))('rejects $kind changes to the pinned $path fixture',({path,original,reviewed,kind})=>{
  if(kind==='production')reviewed='fn injected_runtime() {}\n'+reviewed;
  if(kind==='append')reviewed+='\npub fn injected_runtime() {}\n';
  if(kind==='original')original+='\npub fn injected_runtime() {}\n';
  if(kind==='test'){
   const boundary=reviewed.indexOf(path.includes('login_startup')?'\n    #[cfg(test)]\n    mod tests {':'\n#[cfg(test)]\nmod tests {');
   reviewed=reviewed.slice(0,boundary)+reviewed.slice(boundary).replace('unwrap();','expect("unreviewed");');
  }
  if(kind==='path')path=path.includes('login_startup')?'src-tauri/src/data/archive_recovery.rs':'src-tauri/src/installer/login_startup.rs';
  expect(()=>validateNativeTestOverride(Buffer.from(original),Buffer.from(reviewed),path)).toThrow();
 });
 it('rejects a native override outside the explicit path map',()=>{
  expect(()=>validateNativeTestOverride(Buffer.from(nativeOriginal),Buffer.from(nativeReviewed),'src-tauri/src/lib.rs')).toThrow();
 });
 it('permits a native test fixture delta but rejects production changes and ambiguous boundaries',()=>{
  expect(()=>validateNativeTestOverride(Buffer.from(nativeOriginal),Buffer.from(nativeReviewed))).not.toThrow();
  for(const changed of ['fn runtime() {}\n'+nativeReviewed,nativeReviewed.replace('#[cfg(test)]','#[cfg(any())]'),nativeReviewed+'\n#[cfg(test)]\nmod tests {}'])expect(()=>validateNativeTestOverride(Buffer.from(nativeOriginal),Buffer.from(changed))).toThrow();
 });
 it.each(['append','escape','alter test','alter original'])('rejects unreviewed native suffix changes: %s',kind=>{
  let original=nativeOriginal,reviewed=nativeReviewed;
  if(kind==='append')reviewed+='\npub fn injected_runtime() {}\n';
  if(kind==='escape')reviewed=reviewed.replace('mod tests {','mod tests {}\npub fn injected_runtime() {}\nmod hidden {');
  if(kind==='alter test')reviewed=reviewed.replace('drop(scope);','std::mem::forget(scope);');
  if(kind==='alter original')original+='\npub fn injected_runtime() {}\n';
  expect(()=>validateNativeTestOverride(Buffer.from(original),Buffer.from(reviewed))).toThrow();
 });
 const product='a'.repeat(40),controller='b'.repeat(40),ref='refs/tags/v0.1.9-release-check-e2e-1';
 it('admits only the named meter test path beside the existing check fixtures',()=>{
  expect(()=>validateFrozenController('v0.1.9',product,controller,ref,[meterFixture.path])).not.toThrow();
  expect(()=>validateFrozenController('v0.1.9',product,controller,ref,['tests/voice-other.test.tsx'])).toThrow();
 });
 it('allows the reviewed dictation E2E path without permitting neighboring tests',()=>{
  expect(()=>validateFrozenController('v0.1.9',product,controller,ref,['tests/e2e/dictation.spec.ts'])).not.toThrow();
  expect(()=>validateFrozenController('v0.1.9',product,controller,ref,['tests/e2e/speech.spec.ts'])).toThrow();
 });
 it('permits only the named standalone theme fixture and rejects neighboring bootstrap code',()=>{
  expect(()=>validateFrozenController('v0.1.9',product,controller,ref,['tests/fixtures/voice-recovery.tsx'])).not.toThrow();
  for(const path of ['src/main.tsx','src/design/themes.ts','tests/fixtures/other.tsx'])expect(()=>validateFrozenController('v0.1.9',product,controller,ref,[path])).toThrow();
 });
 it.each(['src/app/App.tsx','src-tauri/src/lib.rs','package.json','.github/workflows/windows-ci.yml','../tests/e2e/models.spec.ts'])('rejects undeclared/product change %s',path=>{
  expect(()=>validateFrozenController('v0.1.9',product,controller,ref,['tests/e2e/models.spec.ts',path])).toThrow();
 });
 it('requires distinct exact identities and the matching diagnostic controller ref',()=>{
  for(const bad of ['refs/heads/main','refs/tags/v0.1.8-release-check-e2e-1','refs/tags/v0.1.9-release-check-'])expect(()=>validateFrozenController('v0.1.9',product,controller,bad,['tests/e2e/models.spec.ts'])).toThrow();
  expect(()=>validateFrozenController('v0.1.9',product,product,ref,['tests/e2e/models.spec.ts'])).toThrow();
  expect(()=>validateFrozenController('v0.1.9',product,'unknown',ref,['tests/e2e/models.spec.ts'])).toThrow();
 });
 it.each([['e2e',0],['e2e',1],['e2e',2],['check',0],['check',1],['check',2],['formal',0],['formal',1],['formal',2]] as const)('preserves the frozen checkout and failure facts for %s outcome %s',async(kind,status)=>{
  const root=mkdtempSync(join(tmpdir(),'beaver-frozen-release-'));
  const owned=resolve(root);if(!owned.startsWith(resolve(tmpdir())+sep)||!owned.split(sep).at(-1)?.startsWith('beaver-frozen-release-'))throw Error('Unexpected temporary fixture path');
  const git=(args:string[])=>execFileSync('git',args,{cwd:root,encoding:'utf8',stdio:['ignore','pipe','pipe']}).trim();
  try{
   mkdirSync(join(root,'scripts'),{recursive:true});mkdirSync(join(root,'tests/e2e'),{recursive:true});mkdirSync(join(root,'tests/fixtures'),{recursive:true});mkdirSync(join(root,'src-tauri/src/installer'),{recursive:true});
   writeFileSync(join(root,'src-tauri/src/installer/record.rs'),nativeOriginal);
   mkdirSync(join(root,'src-tauri/src/data'),{recursive:true});
   for(const fixture of frozenFixtures)writeFileSync(join(root,fixture.path),fixture.original);
   mkdirSync(join(root,'src-tauri/text-service/src'),{recursive:true});
   writeFileSync(join(root,'src-tauri/text-service/src/formal_switch_probe.rs'),'original probe');
   writeFileSync(join(root,'package.json'),JSON.stringify({version:'0.1.9'}));
   writeFileSync(join(root,'scripts/release-validation.mjs'),readFileSync('scripts/release-validation.mjs'));
   writeFileSync(join(root,'scripts/test-richtext-block.mjs'),'original check');
   for(const path of ['models','ui-interaction','dictation'])writeFileSync(join(root,`tests/e2e/${path}.spec.ts`),'original');
   writeFileSync(join(root,'tests/fixtures/voice-recovery.tsx'),'original theme fixture');
   git(['init']);git(['config','user.name','fixture']);git(['config','user.email','fixture@example.invalid']);git(['config','commit.gpgsign','false']);git(['config','core.autocrlf','false']);git(['config','core.hooksPath',join(root,'no-hooks')]);
   git(['add','.']);git(['commit','-m','product']);const product=git(['rev-parse','HEAD']);git(['tag','v0.1.9']);
   for(const path of ['models','ui-interaction','dictation'])writeFileSync(join(root,`tests/e2e/${path}.spec.ts`),'reviewed');
   writeFileSync(join(root,'tests/fixtures/voice-recovery.tsx'),'reviewed theme fixture');
   writeFileSync(join(root,'scripts/test-richtext-block.mjs'),'reviewed check');
   writeFileSync(join(root,'src-tauri/src/installer/record.rs'),nativeReviewed);
   for(const fixture of frozenFixtures)writeFileSync(join(root,fixture.path),fixture.reviewed);
   writeFileSync(join(root,'src-tauri/text-service/src/formal_switch_probe.rs'),'reviewed probe');
   git(['add','.']);git(['commit','-m','controller']);const controller=git(['rev-parse','HEAD']);git(['tag','v0.1.9-release-check-e2e-1']);git(['update-ref','refs/remotes/origin/main',controller]);git(['checkout','--detach',product]);
   const failure=`Reviewed frozen-product ${kind==='e2e'?'E2E':kind} failed`;
   const work=runFrozenReleaseTests(root,{SAYWEAVE_RELEASE_TAG:'v0.1.9',SAYWEAVE_RELEASE_COMMIT:product,SAYWEAVE_CONTROLLER_COMMIT:controller,SAYWEAVE_CONTROLLER_REF:ref},()=>{for(const fixture of frozenFixtures)expect(readFileSync(join(root,fixture.path),'utf8')).toBe(kind==='check'?fixture.reviewed:fixture.original);expect(readFileSync(join(root,'tests/fixtures/voice-recovery.tsx'),'utf8')).toBe(kind==='e2e'?'reviewed theme fixture':'original theme fixture');expect(readFileSync(join(root,'src-tauri/text-service/src/formal_switch_probe.rs'),'utf8')).toBe(kind==='formal'?'reviewed probe':'original probe');expect(readFileSync(join(root,'tests/e2e/dictation.spec.ts'),'utf8')).toBe(kind==='e2e'?'reviewed':'original');expect(readFileSync(join(root,'src-tauri/src/installer/record.rs'),'utf8')).toBe(kind==='check'?nativeReviewed:nativeOriginal);expect(readFileSync(join(root,'tests/e2e/models.spec.ts'),'utf8')).toBe(kind==='e2e'?'reviewed':'original');expect(readFileSync(join(root,'scripts/test-richtext-block.mjs'),'utf8')).toBe(kind==='check'?'reviewed check':'original check');if(status===2)writeFileSync(join(root,'package.json'),JSON.stringify({version:'0.1.9',unexpected:true}));return {status:status===0?0:1};},kind);
   if(status===0)await work;else if(status===1)await expect(work).rejects.toThrow(failure);else{await expect(work).rejects.toMatchObject({name:'AggregateError',errors:[expect.objectContaining({message:failure}),expect.objectContaining({message:'Frozen product checkout was not restored cleanly'})]});expect(git(['status','--porcelain'])).toContain('package.json');git(['restore','--worktree','--','package.json']);}
   for(const path of ['models','ui-interaction','dictation'])expect(readFileSync(join(root,`tests/e2e/${path}.spec.ts`),'utf8')).toBe('original');
   expect(readFileSync(join(root,'tests/fixtures/voice-recovery.tsx'),'utf8')).toBe('original theme fixture');
   expect(readFileSync(join(root,'scripts/test-richtext-block.mjs'),'utf8')).toBe('original check');
   expect(readFileSync(join(root,'src-tauri/src/installer/record.rs'),'utf8')).toBe(nativeOriginal);
   for(const fixture of frozenFixtures)expect(readFileSync(join(root,fixture.path),'utf8')).toBe(fixture.original);
   expect(readFileSync(join(root,'src-tauri/text-service/src/formal_switch_probe.rs'),'utf8')).toBe('original probe');
   expect(git(['status','--porcelain'])).toBe('');expect(git(['rev-parse','HEAD'])).toBe(product);
   if(status===0){git(['update-ref','refs/remotes/origin/main',product]);let invoked=false;await expect(runFrozenReleaseTests(root,{SAYWEAVE_RELEASE_TAG:'v0.1.9',SAYWEAVE_RELEASE_COMMIT:product,SAYWEAVE_CONTROLLER_COMMIT:controller,SAYWEAVE_CONTROLLER_REF:ref},()=>{invoked=true;return {status:0};})).rejects.toThrow();expect(invoked).toBe(false);expect(git(['status','--porcelain'])).toBe('');}
  }finally{
   rmSync(owned,{recursive:true});
  }
 // Each case creates and validates an independent Git repository through many
 // real subprocesses. This is a fixture budget, not a product response deadline.
 },30_000);
});

describe('owner draft and prior verification provenance',()=>{
 const identity=releaseIdentity('0.1.9','v0.1.9','a'.repeat(40));
 it('accepts only the exact empty draft and protects existing assets',()=>{
  const draft={id:42,tag_name:identity.tag,target_commitish:identity.commit,draft:true,prerelease:false,assets:[]};expect(()=>validateOwnedDraft(identity,draft,'42')).not.toThrow();
  for(const change of [{id:43},{tag_name:'v0.1.8'},{target_commitish:'b'.repeat(40)},{draft:false},{prerelease:true},{assets:[{name:'existing.exe'}]}])expect(()=>validateOwnedDraft(identity,{...draft,...change},'42')).toThrow();
 });
 const evidence=()=>({run:{id:99,repository:{full_name:'qinghui316/Sayweave'},path:'.github/workflows/windows-release.yml',event:'workflow_dispatch',status:'completed',conclusion:'failure',head_sha:'b'.repeat(40),head_branch:'v0.1.9-release-check-e2e-1'},jobs:[{id:101,name:'verify',status:'completed',conclusion:'success',steps:['Verify frozen product and landed controller','Run npm run check','Run reviewed E2E against the frozen product and restore its checkout','Run npm run check:distribution'].map(name=>({name,conclusion:'success'}))}]});
 it('reuses a completed successful verification despite a later publication failure',()=>{const e=evidence();expect(()=>validateVerificationRun(identity,e.run,e.jobs,'99')).not.toThrow();});
 it('reuses successful original-tag checks only at the exact product SHA',()=>{
  const e=evidence();e.run.event='push';e.run.head_branch=identity.tag;e.run.head_sha=identity.commit;
  e.jobs[0]!.steps=['Run npm run check','Run npm run test:e2e','Run npm run check:distribution'].map(name=>({name,conclusion:'success'}));
  expect(()=>validateVerificationRun(identity,e.run,e.jobs,'99')).not.toThrow();
  e.run.head_sha='b'.repeat(40);expect(()=>validateVerificationRun(identity,e.run,e.jobs,'99')).toThrow();
  e.run.head_sha=identity.commit;e.jobs[0]!.steps[1]!.conclusion='skipped';expect(()=>validateVerificationRun(identity,e.run,e.jobs,'99')).toThrow();
 });
 it.each(['wrong_repo','wrong_commit','wrong_tag','wrong_workflow','unfinished','failed_verify','skipped_e2e','missing_distribution','ambiguous_job'])('refuses %s verification evidence',kind=>{
  const e=evidence();if(kind==='wrong_repo')e.run.repository.full_name='evil/repo';if(kind==='wrong_commit')e.run.head_sha='unknown';if(kind==='wrong_tag')e.run.head_branch='v0.1.8-release-check-e2e-1';if(kind==='wrong_workflow')e.run.path='.github/workflows/windows-ci.yml';if(kind==='unfinished')e.run.status='in_progress';if(kind==='failed_verify')e.jobs[0]!.conclusion='failure';if(kind==='skipped_e2e')e.jobs[0]!.steps[2]!.conclusion='skipped';if(kind==='missing_distribution')e.jobs[0]!.steps.pop();if(kind==='ambiguous_job')e.jobs.push(e.jobs[0]!);expect(()=>validateVerificationRun(identity,e.run,e.jobs,'99')).toThrow();
 });
});
