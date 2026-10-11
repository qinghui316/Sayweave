// Diagnostic-only installation of an existing signed candidate. Never an owner host.
import {spawn,execFileSync} from 'node:child_process';
import {createHash,randomUUID} from 'node:crypto';
import {mkdirSync,readFileSync,writeFileSync,renameSync,existsSync,unlinkSync} from 'node:fs';
import path from 'node:path';
import {chromium,expect} from '@playwright/test';
import {readPackages,nsisPayloadInventory,nsisHelperIdentity} from './windows-update-packages.mjs';
import {readTools} from './windows-update-artifacts.mjs';
import {boundedTail,safeStartupText,installedStartup,installedState,restartAndReconnect,aclPrelude,aclSnapshotScript,aclRestoreScript,deploymentReading as readDeployment} from './windows-update-startup.mjs';
import {prepareDisposableDebugPolicy} from './windows-update-debug-policy.mjs';
import {assertFrozenProduct} from './frozen-product-scope.mjs';
import {prepareOlderComponent,observeMappedTransition,installerBoundaryCases,releaseOlderComponent,restoreRegistryAcl,cleanupBoundaryDriver} from './windows-product-boundaries.mjs';

if(process.platform!=='win32'||process.arch!=='x64'||process.env.GITHUB_ACTIONS!=='true'||process.env.RUNNER_ENVIRONMENT!=='github-hosted'||process.env.RUNNER_OS!=='Windows')throw Error('Diagnostic installation requires an elevated disposable GitHub-hosted x64 Windows runner.');
const root=process.cwd(),candidate=process.env.STARTUP_CANDIDATE;
const harness=execFileSync('git',['rev-parse','HEAD'],{encoding:'utf8'}).trim();
if(!/^[a-f0-9]{40}$/.test(candidate??''))throw Error('Exact product candidate required.');
const sourceBinding=assertFrozenProduct(root,candidate,harness),changed=sourceBinding.checks.map(entry=>entry.file);
const packages=readPackages('.local/prior-update-packages',candidate);
const recovery=process.argv.includes('--restart-recovery');
const metadata=process.argv.includes('--metadata');
const aclDiagnostic=process.argv.includes('--acl');
const lifecycle=process.argv.includes('--deployment-lifecycle');
const boundaries=process.argv.includes('--product-boundaries');
if(boundaries&&(lifecycle||metadata||recovery||aclDiagnostic))throw Error('Focused product boundaries run alone');
if(lifecycle&&(metadata||recovery||aclDiagnostic))throw Error('Deployment lifecycle requires isolated automatic deployment, without manual component preparation or other diagnoses.');
const tools=metadata?readTools('.local/prior-update-tools',candidate):null;
if(recovery&&!packages.next)throw Error('Recovery diagnosis requires both frozen signed packages.');
const directory=path.join(root,'.local','installed-startup',randomUUID());mkdirSync(directory,{recursive:true});
const report={schemaVersion:1,kind:'installed-startup-diagnostic',candidateCommit:candidate,acceptanceCommit:harness,sourceBinding,environment:process.env.RUNNER_NAME,changedHarnessFiles:changed,passed:false,commands:[],startups:[],packages:packages.manifest};
function persist(){const file=path.join(directory,'receipt.json');writeFileSync(`${file}.tmp`,JSON.stringify(report,null,2)+'\n');renameSync(`${file}.tmp`,file);}
persist();
let browser,owned,child,debugPolicy,componentPreparation;
const exe=path.join(process.env.LOCALAPPDATA,packages.manifest.product,'sayweave.exe');
const dataRoot=path.join(process.env.APPDATA,packages.manifest.identifier,'data');
function stage(value){report.stage=value;persist();console.log(`Installed startup diagnostic: ${value}`);}
function deploymentReading(){
 return readDeployment(path.dirname(exe));
}
async function attemptInstallation(file){
 const attempt={stage:report.stage,installerSha256:createHash('sha256').update(readFileSync(file)).digest('hex'),observations:[]};
 (report.deploymentAttempts??=[]).push(attempt);const seen=new Set();
 const sample=()=>{
  const reading=deploymentReading(),identity=JSON.stringify(reading);
  if(!seen.has(identity)&&attempt.observations.length<64){seen.add(identity);attempt.observations.push({at:new Date().toISOString(),records:reading});persist();}
 };
 sample();const timer=setInterval(sample,200);
 try{await command(file,['/S']);}finally{clearInterval(timer);sample();report.deployment=deploymentReading();persist();}
}
const invoke=(page,command,args={})=>page.evaluate(({command,args})=>window.__TAURI_INTERNALS__.invoke(command,args),{command,args});
const boundaryContext={root,candidate,directory,report,packages,persist,command,componentInventory,connect,closeCurrent,invoke,deploymentReading,attemptInstallation,stage,exe};
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
async function deploymentLifecycle(){
 stage('same-package deployment lifecycle');
 const first=deploymentReading();
 expect(first.user).toMatchObject({phase:'committed',userChoice:null,userInitialized:true,target:{commit:candidate,mainHash:report.payload.sha256,payloadHash:packages.manifest.componentSha256}});
 expect(first.machine).toMatchObject({phase:'committed',target:{commit:candidate,payloadHash:packages.manifest.componentSha256}});
 let page=await connect();
 expect(await invoke(page,'text_service_status')).toMatchObject({enabled:true,deploymentState:'ready',updateAvailable:false,error:null});
 const visuals=report.nativePersonalization=[];
 await page.getByRole('link',{name:'个性化',exact:true}).click();
 for(const [minimum,zoom] of [[false,1],[true,1],[true,1.5]]){
  await invoke(page,'acceptance_main_size',{minimum,zoom});await page.emulateMedia({reducedMotion:minimum?'reduce':'no-preference'});
  await expect(page.getByRole('heading',{name:'个性化',exact:true})).toHaveCount(1);
  const toggle=page.getByRole('switch',{name:'个性化',exact:true});await expect(toggle).toBeVisible();
  const geometry=await toggle.evaluate(el=>{const r=el.getBoundingClientRect();return {rightGutter:innerWidth-r.right,width:innerWidth,height:innerHeight,ratio:devicePixelRatio};});
  expect(geometry.rightGutter).toBeGreaterThanOrEqual(0);expect(geometry.rightGutter).toBeLessThanOrEqual(40);
  const file=`native-personalization-${minimum?'960':'1120'}-${zoom}.png`;
  await page.evaluate(()=>document.fonts.ready);
  // Wry zoom changes CSS viewport metrics. Capture the surface without a CSS
  // clip so the right edge and footer remain in the native viewport image.
  const session=await page.context().newCDPSession(page);
  let pixels;try{pixels=Buffer.from((await session.send('Page.captureScreenshot',{format:'png',fromSurface:true,captureBeyondViewport:false})).data,'base64');}finally{await session.detach();}
  const imageWidth=pixels.readUInt32BE(16),imageHeight=pixels.readUInt32BE(20);
  expect(imageWidth).toBeGreaterThanOrEqual(Math.floor(geometry.width*geometry.ratio)-1);expect(imageHeight).toBeGreaterThanOrEqual(Math.floor(geometry.height*geometry.ratio)-1);
  writeFileSync(path.join(directory,file),pixels);visuals.push({file,...geometry,imageWidth,imageHeight,headingCount:1});persist();
 }
 await invoke(page,'acceptance_main_size',{minimum:false,zoom:1});
 const seeded=await invoke(page,'acceptance_update_seed');expect(seeded.credentialMatches).toBe(true);
 expect(seeded.schema).toBe(9);expect(seeded.history.count).toBe(10);expect(seeded.profile.preferenceCount).toBe(1);
 const cases=report.deploymentLifecycle={first,repeat:[],uninstall:null,reinstall:null,passed:false};persist();
 for(const enabled of [false,true]){
  await invoke(page,'text_service_action',{action:enabled?'enable':'disable'});
  const before=await invoke(page,'acceptance_update_data');expect(before.component.enabled).toBe(enabled);
  await closeCurrent(page);
  try{await attemptInstallation(packages.old);}finally{cases.repeat.push({enabled,records:deploymentReading()});persist();}
  const records=cases.repeat.at(-1).records;
  expect(records.user).toMatchObject({phase:'committed',userChoice:enabled,userInitialized:true,target:{commit:candidate,mainHash:report.payload.sha256}});
  page=await connect();expect(await invoke(page,'acceptance_update_data')).toEqual(before);
  cases.repeat.at(-1).passed=true;persist();
 }
 const before=await invoke(page,'acceptance_update_data');await closeCurrent(page);
 const database=path.join(dataRoot,'dictionary.sqlite'),databaseHash=createHash('sha256').update(readFileSync(database)).digest('hex');
 const original=deploymentReading(),uninstaller=path.join(path.dirname(exe),'uninstall.exe');
 const registrationBefore=await componentInventory();
 const module=path.join(process.env.ProgramFiles,'Sayweave','TextService',packages.manifest.componentSha256,'sayweave_text_service.dll');
 expect(registrationBefore.HKEY_LOCAL_MACHINE).toMatchObject({com:true,tip:true});
 expect(registrationBefore.HKEY_LOCAL_MACHINE.module.replace(/^\\\\\?\\/,'').toLowerCase()).toBe(module.toLowerCase());
 expect(createHash('sha256').update(readFileSync(module)).digest('hex')).toBe(packages.manifest.componentSha256);
 expect(createHash('sha256').update(readFileSync(uninstaller)).digest('hex')).toBe(original.user.target.uninstallHash);
 stage('owned true uninstall retains data, credentials and shared component');
 try{await attemptInstallation(uninstaller);}finally{cases.uninstall={records:deploymentReading()};persist();}
 // NSIS's original uninstaller returns after spawning its temporary copy.
 // Parent exit0 is only launch evidence; wait for the owned durable terminal.
 await expect.poll(()=>{cases.uninstall.records=deploymentReading();persist();return cases.uninstall.records.user.phase;},{timeout:180000,intervals:[100,200,500]}).toBe('uninstalled');
 expect(existsSync(exe)).toBe(false);expect(existsSync(uninstaller)).toBe(false);expect(existsSync(path.join(path.dirname(exe),'sayweave-installer-helper.exe'))).toBe(false);
 for(const file of original.user.files)expect(existsSync(file.path)).toBe(false);
 const keys=JSON.stringify(original.user.keys.map(key=>key.path));
 const inventory=await command('powershell',['-NoProfile','-Command',`$ErrorActionPreference='Stop';$keys=ConvertFrom-Json '${keys.replaceAll("'","''")}';foreach($key in $keys){if(Test-Path -LiteralPath ('Registry::HKEY_CURRENT_USER\\'+$key)){throw 'Owned install namespace remains'}};'removed'`]);
 expect(inventory.trim()).toBe('removed');
 expect(createHash('sha256').update(readFileSync(database)).digest('hex')).toBe(databaseHash);
 const registrationAfter=await componentInventory();expect(registrationAfter).toEqual(registrationBefore);
 expect(createHash('sha256').update(readFileSync(module)).digest('hex')).toBe(packages.manifest.componentSha256);
 const canonical=value=>Array.isArray(value)?value.map(canonical):value&&typeof value==='object'?Object.fromEntries(Object.keys(value).sort().map(key=>[key,canonical(value[key])])):value;
 cases.uninstall.registrationDigest=createHash('sha256').update(JSON.stringify(canonical(registrationAfter))).digest('hex');cases.uninstall.componentSha256=packages.manifest.componentSha256;
 expect(cases.uninstall.records.machine).toEqual(original.machine);cases.uninstall.databaseHash=databaseHash;cases.uninstall.passed=true;persist();
 stage('same-package reinstall verifies retained data and credential identity');
 try{await attemptInstallation(packages.old);}finally{cases.reinstall={records:deploymentReading()};persist();}
 expect(cases.reinstall.records.user).toMatchObject({phase:'committed',userChoice:true});
 page=await connect();expect(await invoke(page,'acceptance_update_data')).toEqual(before);await closeCurrent(page);
 cases.reinstall.passed=true;cases.passed=true;persist();
}
async function componentInventory(timeout=180000){
 const inventory=JSON.parse(await command('powershell',['-NoProfile','-Command',"$ErrorActionPreference='Stop';$items=@{};foreach($scope in @('HKEY_CURRENT_USER','HKEY_LOCAL_MACHINE')){$base='Registry::'+$scope;$com=$base+'\\Software\\Classes\\CLSID\\{CB473CF0-9B4C-482C-BFC2-5527D2379ACA}';$tip=$base+'\\Software\\Microsoft\\CTF\\TIP\\{CB473CF0-9B4C-482C-BFC2-5527D2379ACA}';$item=@{com=(Test-Path -LiteralPath $com);tip=(Test-Path -LiteralPath $tip);module=$null;owner=$null;tipEntries=@()};if(Test-Path -LiteralPath ($com+'\\InprocServer32')){$key=Get-Item -LiteralPath ($com+'\\InprocServer32');$item.module=$key.GetValue('');$item.owner=$key.GetValue('SayweaveOwner')};if($item.tip){$queue=@($tip);for($n=0;$n -lt $queue.Count;$n++){if($queue.Count -gt 64){throw 'Owned TIP registry inventory exceeded bound'};$key=Get-Item -LiteralPath $queue[$n];$values=@{};$names=@($key.GetValueNames());if($names.Count -gt 32){throw 'Owned TIP value inventory exceeded bound'};foreach($name in $names){$value=$key.GetValue($name);if(($value -isnot [string] -and $value -isnot [int]) -or ([string]$value).Length -gt 1024){throw 'Owned TIP value exceeded bound'};$values[$name]=@{kind=([string]$key.GetValueKind($name));value=$value}};$item.tipEntries+=@{path=$queue[$n].Substring($tip.Length);values=$values};foreach($child in @($key.GetSubKeyNames())){$queue+=($queue[$n]+'\\'+$child)}}};$items[$scope]=$item};ConvertTo-Json -InputObject $items -Compress -Depth 9"],timeout));
 if(Object.keys(inventory).sort().join(',')!=='HKEY_CURRENT_USER,HKEY_LOCAL_MACHINE'||Object.values(inventory).some(item=>typeof item.com!=='boolean'||typeof item.tip!=='boolean'))throw Error('Fixed-class inventory shape differs');
 return inventory;
}
// Windows SDK TF_IPP_FLAG_ENABLED=2; active=1 is not required for readiness.
function profileReady(reading){
 const q=reading.query,p=q.managerProfiles;
 return q.class==='CB473CF0-9B4C-482C-BFC2-5527D2379ACA'&&q.expectedProfile==='6CD26440-E770-471E-8212-B5A565E816F5'&&p.length===1&&p[0].clsid===q.class&&p[0].profile===q.expectedProfile&&p[0].category==='B5A73CD1-8355-426B-A161-259808F26B14'&&p[0].type===1&&p[0].language===q.legacyLanguage&&(p[0].flags&2)===2&&q.categories.length===1&&q.categories[0]===p[0].category&&reading.inventory.HKEY_LOCAL_MACHINE.com&&reading.inventory.HKEY_LOCAL_MACHINE.tip&&!reading.inventory.HKEY_CURRENT_USER.com;
}
async function activationReading(timeout=180000){
 const started=Date.now(),query=JSON.parse(await command(tools.probe,['registration-query-detail'],timeout));
 const inventory=await componentInventory(Math.max(1,timeout-(Date.now()-started)));
 return {startedAt:new Date(started).toISOString(),elapsedMs:Date.now()-started,query,inventory};
}
async function installedActivation(label,module){
 const started=Date.now(),result={label,passed:false,startedAt:new Date().toISOString()};
 try{await command(tools.probe,['installed-fixture',module],30000);result.passed=true;}
 catch(error){result.error=safeStartupText(error.message).slice(0,2048);}
 result.elapsedMs=Date.now()-started;return result;
}
async function registeredComparison(module,activation){
 const started=Date.now(),deadline=started+10000,samples=[];let stable=0;
 activation.readiness={samples};persist();
 do{
  const reading=await activationReading(Math.max(1,deadline-Date.now())),ready=profileReady(reading);
  samples.push({...reading,readElapsedMs:reading.elapsedMs,elapsedMs:Date.now()-started,ready});persist();
  stable=ready?stable+1:0;
  if(stable===2&&Date.now()<=deadline){activation.afterReadiness=await installedActivation('after observed registration readiness',module);activation.passed=activation.passed&&activation.afterReadiness.passed;persist();return;}
  await new Promise(resolve=>setTimeout(resolve,Math.max(0,Math.min(250,deadline-Date.now()))));
 }while(Date.now()<deadline);
 throw Error('Registered profile readiness did not settle; see fixed identity samples');
}
async function frozenToolingPrelude(){
 stage('same VM frozen product six-case prelude');
 const productRoot=path.join(directory,'product-prelude');
 if(path.relative(directory,productRoot)!=='product-prelude'||existsSync(productRoot))throw Error('Disposable prelude path differs');
 await command('git',['worktree','add','--detach',productRoot,candidate]);
 if((await command('git',['-C',productRoot,'rev-parse','HEAD'])).trim()!==candidate)throw Error('Prelude checkout differs from frozen product');
 const proofFile=path.join(productRoot,'.local/windows-update-acceptance.json'),prelude=report.toolingPrelude={commit:candidate,componentSha256:tools.manifest.artifacts.component.sha256,passed:false};persist();
 let failure;
 try{await command(process.execPath,[path.join(productRoot,'scripts/test-windows-update.mjs'),'--tools-only','--tools-dir',path.resolve('.local/prior-update-tools')],180000,productRoot);}
 catch(error){failure=error;prelude.error=safeStartupText(error.message).slice(0,2048);persist();}
 try{if(existsSync(proofFile)){const bytes=readFileSync(proofFile);if(bytes.length>4*1024*1024)throw Error('Prelude receipt exceeds bound');const proof=JSON.parse(bytes);writeFileSync(path.join(directory,'tooling-prelude.receipt.log'),bytes);prelude.receipt={file:'tooling-prelude.receipt.log',sha256:createHash('sha256').update(bytes).digest('hex'),size:bytes.length};prelude.checks=proof.checks;prelude.passed=proof.commit===candidate&&proof.kind==='tools'&&proof.passed&&proof.checks.length===6&&!(proof.cleanupErrors??[]).length;persist();}}
 catch(error){prelude.receiptError=safeStartupText(error.message).slice(0,2048);persist();if(!failure)throw error;}
 if(failure)throw failure;
 if(!prelude.passed)throw Error('Frozen same-VM tooling prelude failed');
 // Retain this disposable checkout and its evidence until the runner is discarded.
}
async function cleanupComponent(){
 const registered=execFileSync('powershell',['-NoProfile','-Command',"$key=Get-Item -LiteralPath 'Registry::HKEY_LOCAL_MACHINE\\Software\\Classes\\CLSID\\{CB473CF0-9B4C-482C-BFC2-5527D2379ACA}\\InprocServer32' -ErrorAction Stop;@{module=$key.GetValue('');owner=$key.GetValue('SayweaveOwner')}|ConvertTo-Json -Compress"],{encoding:'utf8'});
 const identity=JSON.parse(registered),expected=path.join(process.env.ProgramFiles,'Sayweave','TextService',tools.manifest.artifacts.component.sha256,'sayweave_text_service.dll');
 if(identity.owner!=='SayweaveStageA-v1'||path.normalize(identity.module.replace(/^\\\\\?\\/,'')).toLowerCase()!==path.normalize(expected).toLowerCase()||createHash('sha256').update(readFileSync(identity.module)).digest('hex')!==tools.manifest.artifacts.component.sha256)throw Error('Refusing cleanup of changed component identity');
 await command(exe,['--sayweave-text-service=remove',`--initiator=${componentPreparation.sid}`]);
 const started=Date.now(),deadline=started+10000,samples=[];let stable=0;
 componentPreparation.cleanup={samples,removed:false};persist();
 do{
  const profiles=JSON.parse(await command(tools.probe,['registration-state'],Math.max(1,deadline-Date.now())));
  const inventory=await componentInventory(Math.max(1,deadline-Date.now()));
  samples.push({elapsedMs:Date.now()-started,inventory,profiles});persist();
  if(Object.values(inventory).some(item=>item.com||item.tip))throw Error('Native removal left fixed-class registry roots; no diagnostic deletion attempted');
  const matches=profiles.profiles===0&&profiles.categories===0&&profiles.speechCategory===false;
  stable=matches?stable+1:0;
  if(stable===2&&Date.now()<=deadline){componentPreparation.cleanup.removed=true;componentPreparation.removed=true;return;}
  await new Promise(resolve=>setTimeout(resolve,Math.max(0,Math.min(250,deadline-Date.now()))));
 }while(Date.now()<deadline);
 throw Error('Prepared component cleanup did not settle; see bounded independent-process samples');
}
async function command(file,args,timeout=180000,cwd=root){
 if(!Number.isSafeInteger(timeout)||timeout<1||timeout>180000)throw Error('Diagnostic command timeout exceeds bound');
 const entry={tool:path.basename(file),args,stage:report.stage,startedAt:new Date().toISOString()};report.commands.push(entry);persist();
 let stdout=Buffer.alloc(0),stderr=Buffer.alloc(0),error;
 const processChild=spawn(file,args,{cwd,windowsHide:true,stdio:['ignore','pipe','pipe'],timeout});
 entry.pid=processChild.pid;entry.timeoutMs=timeout;persist();
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
 report.payloadInventory=nsisPayloadInventory(await command(sevenZip,['l','-slt',packages.old]));persist();
 await command(sevenZip,['x',packages.old,`-o${extracted}`,'sayweave.exe','sayweave-installer-helper.exe','$PLUGINSDIR\\sayweave-maintenance.exe','$PLUGINSDIR\\sayweave-main.exe','$PLUGINSDIR\\sayweave-uninstall.exe','-y']);
 boundaryContext.extracted=extracted;
 report.helperIdentity=nsisHelperIdentity(readFileSync(path.join(extracted,'sayweave-installer-helper.exe')),readFileSync(path.join(extracted,'$PLUGINSDIR','sayweave-maintenance.exe')));persist();
 const payload=readFileSync(path.join(extracted,'sayweave.exe'));
 if(payload.length<1||payload.length>128*1024*1024)throw Error('Signed executable payload exceeds bound.');
 const hash=bytes=>createHash('sha256').update(bytes).digest('hex');
 report.payload={sha256:hash(payload),size:payload.length,manifestSha256:packages.manifest.artifacts.old.executableSha256,manifestConsistent:hash(payload)===packages.manifest.artifacts.old.executableSha256};persist();
 for(const value of [candidate,packages.manifest.identifier,packages.manifest.url,packages.manifest.publicKey])if(!payload.includes(Buffer.from(value)))throw Error('Frozen identity missing from signed executable.');
 if(installedState(exe).applications.length||existsSync(exe)||existsSync(dataRoot))throw Error('Isolated candidate installation is not empty.');
 stage('read-only WebView policy diagnostic');
 const policy=await command('powershell',['-NoProfile','-Command',"[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false);$items=@();foreach($scope in @('HKLM:','HKCU:')){foreach($suffix in @('Software\\Policies\\Microsoft\\Edge','Software\\Policies\\Microsoft\\Edge\\WebView2\\AdditionalBrowserArguments')){$key=$scope+'\\'+$suffix;if(Test-Path -LiteralPath $key){$entry=Get-Item -LiteralPath $key;foreach($name in @('RemoteDebuggingAllowed','sayweave.exe','*')){if($entry.GetValueNames() -contains $name){$value=$entry.GetValue($name);$items+=@{scope=$scope;key=$suffix;name=$name;kind=[string]$entry.GetValueKind($name);debugAllowed=if($name -eq 'RemoteDebuggingAllowed'){$value -eq 1}else{$null};hasDebugPort=([string]$value -match '--remote-debugging-port');hasLoopback=([string]$value -match '--remote-debugging-address=127.0.0.1')}}}}}};ConvertTo-Json -InputObject $items -Compress"]);
 report.debugPolicy=JSON.parse(policy);persist();
 if(metadata)await frozenToolingPrelude();
 if(boundaries){stage('historical production component with a live COM lease');await prepareOlderComponent(boundaryContext);}
 stage('same candidate installation');await attemptInstallation(packages.old);
 if(boundaries)await observeMappedTransition(boundaryContext);
 if(hash(readFileSync(exe))!==report.payload.sha256)throw Error('Installed executable differs from signed payload.');
 if(metadata){
  stage('frozen installed component preparation');
  const sid=execFileSync('powershell',['-NoProfile','-Command','[Security.Principal.WindowsIdentity]::GetCurrent().User.Value'],{encoding:'utf8'}).trim();
  const prestate=execFileSync('powershell',['-NoProfile','-Command',"$ErrorActionPreference='Stop';foreach($root in @('HKCU:','HKLM:')){foreach($key in @('Software\\Classes\\CLSID\\{CB473CF0-9B4C-482C-BFC2-5527D2379ACA}','Software\\Microsoft\\CTF\\TIP\\{CB473CF0-9B4C-482C-BFC2-5527D2379ACA}')){if(Test-Path -LiteralPath ($root+'\\'+$key)){throw 'Existing component registration'}}};'empty'"],{encoding:'utf8'}).trim();
  if(prestate!=='empty'||tools.manifest.artifacts.component.sha256!==packages.manifest.componentSha256)throw Error('Metadata component prestate/identity differs');
  componentPreparation={sid,prestate,attempted:true};report.componentPreparation=componentPreparation;persist();
  await command(exe,['--sayweave-text-service=install',`--initiator=${sid}`]);
  const module=execFileSync('powershell',['-NoProfile','-Command',"(Get-Item -LiteralPath 'Registry::HKEY_LOCAL_MACHINE\\Software\\Classes\\CLSID\\{CB473CF0-9B4C-482C-BFC2-5527D2379ACA}\\InprocServer32').GetValue('')"],{encoding:'utf8'}).trim();
  componentPreparation.module=module;componentPreparation.sha256=hash(readFileSync(module));if(componentPreparation.sha256!==tools.manifest.artifacts.component.sha256)throw Error('Prepared module differs from frozen tooling');persist();
  stage('frozen installed component activation characterization');
  const activation=report.componentActivation={passed:false,before:await activationReading()};activation.before.ready=profileReady(activation.before);persist();
  activation.immediate=await installedActivation('first activation after minimal fixed identity sample',module);activation.passed=activation.immediate.passed;persist();
  activation.after=await activationReading();persist();
  if(!activation.before.ready||!activation.immediate.passed)await registeredComparison(module,activation);
  stage('frozen ordinary component metadata characterization after activation');
  await command('powershell',['-NoProfile','-ExecutionPolicy','Bypass','-File','scripts/windows-update-metadata.ps1','-ExpectedCommit',harness,'-ProductCommit',candidate,'-ExpectedModulePath',module,'-ExpectedComponentSha256',tools.manifest.artifacts.component.sha256]);
  report.metadataCharacterized=true;persist();
 }
 stage('temporary elevated WebView debug policy');
 debugPolicy=prepareDisposableDebugPolicy(9291);report.debugPolicyLease=debugPolicy.receipt;persist();debugPolicy.install();persist();
 stage('process/window/page/debug/native renderer');
 let page=await connect();
 await page.screenshot({path:path.join(directory,'native-startup.png')});
 await closeCurrent(page);
 if(lifecycle)await deploymentLifecycle();
 if(boundaries)await installerBoundaryCases(boundaryContext);
 if(aclDiagnostic){
  stage('PowerShell ACL module comparison');
  const literal="'"+path.dirname(exe).replaceAll("'","''")+"'";
  const inspect=String.raw`[Console]::OutputEncoding=[System.Text.UTF8Encoding]::new($false);$r=@{version=$PSVersionTable.PSVersion.ToString();edition=$PSVersionTable.PSEdition;pshome=$PSHOME;modulePath=@($env:PSModulePath -split ';');available=@(Get-Module -ListAvailable Microsoft.PowerShell.Security|ForEach-Object{@{version=$_.Version.ToString();path=$_.Path}});loaded=@();read=$false};try{Import-Module Microsoft.PowerShell.Security -ErrorAction Stop;$null=Get-Acl -LiteralPath `+literal+String.raw` -ErrorAction Stop;$r.read=$true}catch{$r.error=$_.Exception.ToString();$r.errorId=$_.FullyQualifiedErrorId};$r.loaded=@(Get-Module Microsoft.PowerShell.Security|ForEach-Object{@{version=$_.Version.ToString();path=$_.Path}});ConvertTo-Json -InputObject $r -Compress -Depth 5`;
  report.aclModuleDefault=JSON.parse(await command('powershell',['-NoProfile','-Command',inspect]));persist();
  report.aclModuleExplicit=JSON.parse(await command('powershell',['-NoProfile','-Command',aclPrelude+inspect.replace('Import-Module Microsoft.PowerShell.Security -ErrorAction Stop;','')]));persist();
  expect(report.aclModuleExplicit.read).toBe(true);
  stage('owned installation ACL denial, read comparison and exact restoration');
  const scratch=path.dirname(exe),beforeHash=hash(readFileSync(exe));expect(beforeHash).toBe(packages.manifest.artifacts.old.executableSha256);
  const original=JSON.parse(await command('powershell',['-NoProfile','-Command',aclSnapshotScript(scratch)]));if(!original.binary)throw Error('Owned scratch ACL is empty');aclRestoreScript(scratch,original);
  report.aclRoundtrip={directory:scratch,original,beforeHash,denied:false,restored:false};persist();
  const user=execFileSync('whoami',[],{encoding:'utf8'}).trim();
  try{
   await command('icacls',[scratch,'/deny',`${user}:(OI)(CI)(W,D)`]);
   try{report.aclRoundtrip.readDuringDenial={succeeded:true,sha256:hash(readFileSync(exe))};}catch(error){report.aclRoundtrip.readDuringDenial={succeeded:false,code:error.code??'unknown',message:safeStartupText(error.message).slice(0,1024)};}persist();
   try{writeFileSync(path.join(scratch,'must-not-create.tmp'),'forbidden');}catch{report.aclRoundtrip.denied=true;}persist();expect(report.aclRoundtrip.denied).toBe(true);
  }finally{
   report.aclRoundtrip.after=JSON.parse(await command('powershell',['-NoProfile','-Command',aclRestoreScript(scratch,original)]));report.aclRoundtrip.restored=report.aclRoundtrip.after.descriptorMatches;persist();expect(report.aclRoundtrip.restored).toBe(true);
   const proof=path.join(scratch,'restored-write.tmp');writeFileSync(proof,'restored');unlinkSync(proof);report.aclRoundtrip.writeRestored=true;persist();
   report.aclRoundtrip.afterHash=hash(readFileSync(exe));persist();expect(report.aclRoundtrip.afterHash).toBe(beforeHash);
  }
  report.aclCompatibilityPassed=true;persist();
 }
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
 if(metadata&&!report.componentActivation.passed)throw Error('Frozen installed component activation failed; native UI proof is separate; see preserved probe streams');
 report.passed=true;
}catch(error){report.error=safeStartupText(error?.message??error?.code??String(error)).slice(0,2048);process.exitCode=1;}
finally{
 if(boundaries){try{await restoreRegistryAcl(boundaryContext);await cleanupBoundaryDriver(boundaryContext);await releaseOlderComponent(boundaryContext);}catch(error){report.boundaryCleanupError=String(error.message).slice(0,1024);report.passed=false;process.exitCode=1;}}
 if(browser)await browser.close().catch(()=>{});
 if(owned){try{const current=installedState(exe).applications.find(app=>app.ProcessId===owned.ProcessId&&app.CreationDate===owned.CreationDate);if(current){report.cleanupKill={ProcessId:current.ProcessId,CreationDate:current.CreationDate};await command('taskkill',['/PID',String(current.ProcessId),'/T','/F']);}}catch(error){report.cleanupError=String(error.message).slice(0,1024);report.passed=false;process.exitCode=1;}}
 else if(child&&child.exitCode===null){report.unverifiedProcessCleanup=true;report.passed=false;process.exitCode=1;}
 if(componentPreparation)await cleanupComponent().catch(error=>{report.componentCleanupError=safeStartupText(error.message).slice(0,1024);report.passed=false;process.exitCode=1;});
 if(debugPolicy){try{debugPolicy.remove();}catch(error){report.policyCleanupError=safeStartupText(error.message).slice(0,1024);report.passed=false;process.exitCode=1;}}
 persist();console.log(JSON.stringify({kind:report.kind,candidateCommit:candidate,acceptanceCommit:harness,passed:report.passed,error:report.error,receipt:path.join(directory,'receipt.json')}));
}
