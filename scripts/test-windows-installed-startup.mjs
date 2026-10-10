// Diagnostic-only installation of an existing signed candidate. Never an owner host.
import {spawn,execFileSync} from 'node:child_process';
import {createHash,randomUUID} from 'node:crypto';
import {mkdirSync,readFileSync,writeFileSync,renameSync,existsSync} from 'node:fs';
import path from 'node:path';
import {chromium,expect} from '@playwright/test';
import {readPackages} from './windows-update-packages.mjs';
import {readTools} from './windows-update-artifacts.mjs';
import {boundedTail,safeStartupText,installedStartup,installedState,restartAndReconnect} from './windows-update-startup.mjs';
import {prepareDisposableDebugPolicy} from './windows-update-debug-policy.mjs';

if(process.platform!=='win32'||process.arch!=='x64'||process.env.GITHUB_ACTIONS!=='true'||process.env.RUNNER_ENVIRONMENT!=='github-hosted'||process.env.RUNNER_OS!=='Windows')throw Error('Diagnostic installation requires an elevated disposable GitHub-hosted x64 Windows runner.');
const root=process.cwd(),candidate=process.env.STARTUP_CANDIDATE;
const harness=execFileSync('git',['rev-parse','HEAD'],{encoding:'utf8'}).trim();
if(!/^[a-f0-9]{40}$/.test(candidate??''))throw Error('Exact product candidate required.');
const permitted=new Set(['.github/workflows/windows-ci.yml','scripts/windows-update-startup.mjs','scripts/windows-update-debug-policy.mjs','scripts/windows-update-metadata.ps1','scripts/windows-update-metadata.cs','scripts/test-windows-installed-startup.mjs','scripts/check-distribution.mjs','scripts/test-windows-update.mjs','docs/release.md','docs/testing.md','tests/windows-update-startup.test.ts']);
const changed=execFileSync('git',['diff','--name-only',candidate,harness],{encoding:'utf8'}).trim().split('\n').filter(Boolean);
if(changed.some(file=>!permitted.has(file)))throw Error(`Candidate product sources differ: ${changed.filter(file=>!permitted.has(file)).join(', ')}`);
if(execFileSync('git',['status','--porcelain','--untracked-files=no'],{encoding:'utf8'}).trim())throw Error('Diagnostic source tree is dirty.');
const packages=readPackages('.local/prior-update-packages',candidate);
const recovery=process.argv.includes('--restart-recovery');
const metadata=process.argv.includes('--metadata');
const tools=metadata?readTools('.local/prior-update-tools',candidate):null;
if(recovery&&!packages.next)throw Error('Recovery diagnosis requires both frozen signed packages.');
const directory=path.join(root,'.local','installed-startup',randomUUID());mkdirSync(directory,{recursive:true});
const report={schemaVersion:1,kind:'installed-startup-diagnostic',candidateCommit:candidate,acceptanceCommit:harness,environment:process.env.RUNNER_NAME,changedHarnessFiles:changed,passed:false,commands:[],startups:[],packages:packages.manifest};
function persist(){const file=path.join(directory,'receipt.json');writeFileSync(`${file}.tmp`,JSON.stringify(report,null,2)+'\n');renameSync(`${file}.tmp`,file);}
persist();
let browser,owned,child,debugPolicy,componentPreparation;
const exe=path.join(process.env.LOCALAPPDATA,packages.manifest.product,'sayweave.exe');
const dataRoot=path.join(process.env.APPDATA,packages.manifest.identifier,'data');
function stage(value){report.stage=value;persist();console.log(`Installed startup diagnostic: ${value}`);}
const invoke=(page,command,args={})=>page.evaluate(({command,args})=>window.__TAURI_INTERNALS__.invoke(command,args),{command,args});
async function connect(launch=true){
 const result=await installedStartup({exe,dataRoot,directory,port:9291,launch,previous:owned,record:report.startups,persist,onOwned:value=>{owned=value;},onSpawn:value=>{child=value;}});
 browser=await chromium.connectOverCDP(result.endpoint.webSocketDebuggerUrl,{timeout:10000});
 let page;for(let n=0;n<100;n++){page=browser.contexts().flatMap(context=>context.pages()).find(p=>p.url().includes('tauri')&&!p.url().includes('surface=overlay'));if(page)break;await new Promise(resolve=>setTimeout(resolve,100));}
 if(!page)throw Error('Main page missing.');
 await page.waitForFunction(()=>window.__TAURI_INTERNALS__?.invoke,{},{timeout:10000});await expect(page.locator('aside')).toBeVisible();
 const snapshot=await invoke(page,'update_snapshot');expect(snapshot.build).toMatchObject({commit:candidate,version:'0.1.700',channel:'test'});report.build=snapshot.build;
 result.entry.rendered=true;result.entry.phase='rendererReady';persist();return page;
}
async function closeCurrent(page){
 await invoke(page,'acceptance_update_exit').catch(error=>{if(!String(error.message).includes('Target page, context or browser has been closed'))throw error;});
 await browser.close();browser=null;
 for(let n=0;n<20;n++){if(!installedState(exe).applications.some(app=>app.CreationDate===owned.CreationDate))break;await new Promise(resolve=>setTimeout(resolve,250));}
 if(installedState(exe).applications.some(app=>app.CreationDate===owned.CreationDate))throw Error('Native application exit did not complete.');
}
async function cleanupComponent(){
 const registered=execFileSync('powershell',['-NoProfile','-Command',"$key=Get-Item -LiteralPath 'Registry::HKEY_LOCAL_MACHINE\\Software\\Classes\\CLSID\\{CB473CF0-9B4C-482C-BFC2-5527D2379ACA}\\InprocServer32' -ErrorAction Stop;@{module=$key.GetValue('');owner=$key.GetValue('SayweaveOwner')}|ConvertTo-Json -Compress"],{encoding:'utf8'});
 const identity=JSON.parse(registered),expected=path.join(process.env.ProgramFiles,'Sayweave','TextService',tools.manifest.artifacts.component.sha256,'sayweave_text_service.dll');
 if(identity.owner!=='SayweaveStageA-v1'||path.normalize(identity.module.replace(/^\\\\\?\\/,'')).toLowerCase()!==path.normalize(expected).toLowerCase()||createHash('sha256').update(readFileSync(identity.module)).digest('hex')!==tools.manifest.artifacts.component.sha256)throw Error('Refusing cleanup of changed component identity');
 await command(exe,['--sayweave-text-service=remove',`--initiator=${componentPreparation.sid}`]);
 const inventory=JSON.parse(await command('powershell',['-NoProfile','-Command',"$ErrorActionPreference='Stop';$items=@{};foreach($scope in @('HKEY_CURRENT_USER','HKEY_LOCAL_MACHINE')){$base='Registry::'+$scope;$com=$base+'\\Software\\Classes\\CLSID\\{CB473CF0-9B4C-482C-BFC2-5527D2379ACA}';$tip=$base+'\\Software\\Microsoft\\CTF\\TIP\\{CB473CF0-9B4C-482C-BFC2-5527D2379ACA}';$item=@{com=(Test-Path -LiteralPath $com);tip=(Test-Path -LiteralPath $tip);module=$null;owner=$null;tipEntries=@()};if(Test-Path -LiteralPath ($com+'\\InprocServer32')){$key=Get-Item -LiteralPath ($com+'\\InprocServer32');$item.module=$key.GetValue('');$item.owner=$key.GetValue('SayweaveOwner')};if($item.tip){$queue=@($tip);for($n=0;$n -lt $queue.Count;$n++){if($queue.Count -gt 64){throw 'Owned TIP registry inventory exceeded bound'};$key=Get-Item -LiteralPath $queue[$n];$values=@{};$names=@($key.GetValueNames());if($names.Count -gt 32){throw 'Owned TIP value inventory exceeded bound'};foreach($name in $names){$value=$key.GetValue($name);if(($value -isnot [string] -and $value -isnot [int]) -or ([string]$value).Length -gt 1024){throw 'Owned TIP value exceeded bound'};$values[$name]=@{kind=([string]$key.GetValueKind($name));value=$value}};$item.tipEntries+=@{path=$queue[$n].Substring($tip.Length);values=$values};foreach($child in @($key.GetSubKeyNames())){$queue+=($queue[$n]+'\\'+$child)}}};$items[$scope]=$item};ConvertTo-Json -InputObject $items -Compress -Depth 9"]));
 const profiles=JSON.parse(await command(tools.probe,['registration-state']));
 componentPreparation.cleanup={inventory,profiles};persist();
 if(Object.values(inventory).some(item=>item.com||item.tip)||profiles.profiles!==0||profiles.categories!==0||profiles.speechCategory!==false)throw Error('Prepared component cleanup incomplete; see recorded fixed-class inventory and API state');
 componentPreparation.removed=true;
}
async function command(file,args){
 const entry={tool:path.basename(file),args,stage:report.stage,startedAt:new Date().toISOString()};report.commands.push(entry);persist();
 let stdout=Buffer.alloc(0),stderr=Buffer.alloc(0),error;
 const processChild=spawn(file,args,{cwd:root,windowsHide:true,stdio:['ignore','pipe','pipe'],timeout:180000});
 processChild.stdout.on('data',bytes=>{stdout=boundedTail(stdout,bytes);});processChild.stderr.on('data',bytes=>{stderr=boundedTail(stderr,bytes);});processChild.once('error',value=>{error=value;});
 const started=Date.now();entry.exitCode=await new Promise(resolve=>processChild.once('close',resolve));entry.elapsedMs=Date.now()-started;
 for(const [name,bytes] of [['stdout',stdout],['stderr',stderr]]){const fileName=`command-${report.commands.length}.${name}.log`;writeFileSync(path.join(directory,fileName),safeStartupText(bytes.toString('utf8')));entry[name]=fileName;}
 if(error)entry.error=error.code??error.message;persist();
 if(error||processChild.killed||entry.exitCode!==0)throw Error(`Diagnostic command failed: ${entry.tool}; exit=${entry.exitCode}; see bounded streams`);
 return stdout.toString('utf8');
}
try{
 stage('elevated disposable environment');
 await command('powershell',['-NoProfile','-Command',"$p=New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent());if(-not $p.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)){throw 'Disposable runner is not elevated'}"]);
 stage('shared signature verifier');
 // Compile only the existing shared verifier, never the application or Tauri.
 // The two included files are identical to the frozen product candidate.
 const verifier=path.join(directory,'verifier');mkdirSync(verifier);
 writeFileSync(path.join(verifier,'Cargo.toml'),'[package]\nname="installed-startup-signature"\nversion="0.0.0"\nedition="2024"\n[dependencies]\nbase64="=0.22.1"\nminisign-verify="=0.2.5"\n[[bin]]\nname="verify"\npath="main.rs"\n');
 const signatureSource=path.join(root,'src-tauri/src/update_signature.rs').replaceAll('\\','/'),mainSource=path.join(root,'src-tauri/src/bin/verify_update_signature.rs').replaceAll('\\','/');
 writeFileSync(path.join(verifier,'main.rs'),`#[path=${JSON.stringify(signatureSource)}] pub mod update_signature;\nmod sayweave_lib {pub use crate::update_signature;}\ninclude!(${JSON.stringify(mainSource)});\n`);
 await command('cargo',['run','--quiet','--manifest-path',path.join(verifier,'Cargo.toml'),'--',packages.manifest.publicKey,packages.old,`${packages.old}.sig`,'0.1.700']);
 if(recovery)await command('cargo',['run','--quiet','--manifest-path',path.join(verifier,'Cargo.toml'),'--',packages.manifest.publicKey,packages.next,`${packages.next}.sig`,'0.1.701']);
 report.signatureVerified=true;
 stage('signed installer static payload');
 const sevenZip=path.join(process.env.ProgramFiles,'7-Zip','7z.exe');
 if(!existsSync(sevenZip))throw Error('Disposable runner lacks the static 7-Zip extractor; no installation was attempted.');
 const extracted=path.join(directory,'payload');
 await command(sevenZip,['x',packages.old,`-o${extracted}`,'sayweave.exe','-y']);
 const payload=readFileSync(path.join(extracted,'sayweave.exe'));
 if(payload.length<1||payload.length>128*1024*1024)throw Error('Signed executable payload exceeds bound.');
 const hash=bytes=>createHash('sha256').update(bytes).digest('hex');
 report.payload={sha256:hash(payload),size:payload.length,manifestSha256:packages.manifest.artifacts.old.executableSha256,manifestConsistent:hash(payload)===packages.manifest.artifacts.old.executableSha256};persist();
 for(const value of [candidate,packages.manifest.identifier,packages.manifest.url,packages.manifest.publicKey])if(!payload.includes(Buffer.from(value)))throw Error('Frozen identity missing from signed executable.');
 if(installedState(exe).applications.length||existsSync(exe)||existsSync(dataRoot))throw Error('Isolated candidate installation is not empty.');
 stage('read-only WebView policy diagnostic');
 const policy=await command('powershell',['-NoProfile','-Command',"[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false);$items=@();foreach($scope in @('HKLM:','HKCU:')){foreach($suffix in @('Software\\Policies\\Microsoft\\Edge','Software\\Policies\\Microsoft\\Edge\\WebView2\\AdditionalBrowserArguments')){$key=$scope+'\\'+$suffix;if(Test-Path -LiteralPath $key){$entry=Get-Item -LiteralPath $key;foreach($name in @('RemoteDebuggingAllowed','sayweave.exe','*')){if($entry.GetValueNames() -contains $name){$value=$entry.GetValue($name);$items+=@{scope=$scope;key=$suffix;name=$name;kind=[string]$entry.GetValueKind($name);debugAllowed=if($name -eq 'RemoteDebuggingAllowed'){$value -eq 1}else{$null};hasDebugPort=([string]$value -match '--remote-debugging-port');hasLoopback=([string]$value -match '--remote-debugging-address=127.0.0.1')}}}}}};ConvertTo-Json -InputObject $items -Compress"]);
 report.debugPolicy=JSON.parse(policy);persist();
 stage('same candidate installation');await command(packages.old,['/S']);
 if(hash(readFileSync(exe))!==report.payload.sha256)throw Error('Installed executable differs from signed payload.');
 if(metadata){
  stage('frozen ordinary component metadata characterization');
  const sid=execFileSync('powershell',['-NoProfile','-Command','[Security.Principal.WindowsIdentity]::GetCurrent().User.Value'],{encoding:'utf8'}).trim();
  const prestate=execFileSync('powershell',['-NoProfile','-Command',"$ErrorActionPreference='Stop';foreach($root in @('HKCU:','HKLM:')){foreach($key in @('Software\\Classes\\CLSID\\{CB473CF0-9B4C-482C-BFC2-5527D2379ACA}','Software\\Microsoft\\CTF\\TIP\\{CB473CF0-9B4C-482C-BFC2-5527D2379ACA}')){if(Test-Path -LiteralPath ($root+'\\'+$key)){throw 'Existing component registration'}}};'empty'"],{encoding:'utf8'}).trim();
  if(prestate!=='empty'||tools.manifest.artifacts.component.sha256!==packages.manifest.componentSha256)throw Error('Metadata component prestate/identity differs');
  componentPreparation={sid,prestate,attempted:true};report.componentPreparation=componentPreparation;persist();
  await command(exe,['--sayweave-text-service=install',`--initiator=${sid}`]);
  const module=execFileSync('powershell',['-NoProfile','-Command',"(Get-Item -LiteralPath 'Registry::HKEY_LOCAL_MACHINE\\Software\\Classes\\CLSID\\{CB473CF0-9B4C-482C-BFC2-5527D2379ACA}\\InprocServer32').GetValue('')"],{encoding:'utf8'}).trim();
  componentPreparation.module=module;componentPreparation.sha256=hash(readFileSync(module));if(componentPreparation.sha256!==tools.manifest.artifacts.component.sha256)throw Error('Prepared module differs from frozen tooling');persist();
  await command('powershell',['-NoProfile','-ExecutionPolicy','Bypass','-File','scripts/windows-update-metadata.ps1','-ExpectedCommit',harness,'-ProductCommit',candidate,'-ExpectedModulePath',module,'-ExpectedComponentSha256',tools.manifest.artifacts.component.sha256]);
  report.metadataCharacterized=true;persist();
 }
 stage('temporary elevated WebView debug policy');
 debugPolicy=prepareDisposableDebugPolicy(9291);report.debugPolicyLease=debugPolicy.receipt;persist();debugPolicy.install();persist();
 stage('process/window/page/debug/native renderer');
 let page=await connect();
 await page.screenshot({path:path.join(directory,'native-startup.png')});
 await closeCurrent(page);
 if(recovery){
  stage('signed cache fixture for restart boundary only');
  if(!report.payload.manifestConsistent)throw Error('Recovery fixture requires consistent signed payload identity');
  const bytes=readFileSync(packages.next),cache=path.join(dataRoot,'update-cache');
  writeFileSync(path.join(cache,'candidate.exe'),bytes);
  const journal={schemaVersion:1,stage:'ready',candidate:{version:'0.1.701',signature:readFileSync(`${packages.next}.sig`,'utf8').trim(),url:`https://localhost:${packages.manifest.port}/package.exe`,installerSize:bytes.length,installerSha512:createHash('sha512').update(bytes).digest('base64'),notes:'Synthetic signed-cache fixture; restart diagnosis only.'}};
  writeFileSync(path.join(cache,'transaction.json'),JSON.stringify(journal));report.recoveryFixture={synthetic:true,installerSha256:packages.manifest.artifacts.next.sha256};persist();
  page=await connect();const snapshot=await invoke(page,'update_snapshot');expect(snapshot.state).toBe('readyToInstall');
  const before=await invoke(page,'acceptance_update_seed');expect(before.credentialMatches).toBe(true);expect(before.schema).toBe(9);expect(before.history.count).toBe(10);expect(before.profile.preferenceCount).toBe(1);
  stage('real stop timeout then restart current version');
  await invoke(page,'acceptance_update_hold',{kind:'stop'});await invoke(page,'update_install',{taskId:snapshot.taskId,revision:snapshot.revision});
  await expect.poll(async()=>(await invoke(page,'update_snapshot')).state,{timeout:15000}).toBe('recovery');
  page=await restartAndReconnect({original:owned,invoke:()=>invoke(page,'update_restart'),disconnect:async()=>{await browser.close().catch(()=>{});browser=null;},inspect:()=>installedState(exe).applications,reconnect:()=>connect(false),current:()=>owned,record:report.restarts??(report.restarts=[]),persist});
  const after=await invoke(page,'update_snapshot');expect(after.state).toBe('readyToInstall');expect(after.build.version).toBe('0.1.700');expect(await invoke(page,'acceptance_update_data')).toEqual(before);
  report.recoveryPassed=true;persist();await closeCurrent(page);
 }
 report.passed=true;
}catch(error){report.error=safeStartupText(error?.message??error?.code??String(error)).slice(0,2048);process.exitCode=1;}
finally{
 if(browser)await browser.close().catch(()=>{});
 if(owned){try{const current=installedState(exe).applications.find(app=>app.ProcessId===owned.ProcessId&&app.CreationDate===owned.CreationDate);if(current){report.cleanupKill={ProcessId:current.ProcessId,CreationDate:current.CreationDate};await command('taskkill',['/PID',String(current.ProcessId),'/T','/F']);}}catch(error){report.cleanupError=String(error.message).slice(0,1024);report.passed=false;process.exitCode=1;}}
 else if(child&&child.exitCode===null){report.unverifiedProcessCleanup=true;report.passed=false;process.exitCode=1;}
 if(componentPreparation)await cleanupComponent().catch(error=>{report.componentCleanupError=safeStartupText(error.message).slice(0,1024);report.passed=false;process.exitCode=1;});
 if(debugPolicy){try{debugPolicy.remove();}catch(error){report.policyCleanupError=safeStartupText(error.message).slice(0,1024);report.passed=false;process.exitCode=1;}}
 persist();console.log(JSON.stringify({kind:report.kind,candidateCommit:candidate,acceptanceCommit:harness,passed:report.passed,error:report.error,receipt:path.join(directory,'receipt.json')}));
}
