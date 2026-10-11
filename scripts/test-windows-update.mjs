// Real Windows/HTTPS/minisign/NSIS acceptance; only an elevated disposable Windows VM.
import {spawn,execFileSync} from 'node:child_process';
import {randomUUID,createHash} from 'node:crypto';
import {createServer} from 'node:https';
import {mkdirSync,readFileSync,writeFileSync,existsSync,symlinkSync,rmSync,rmdirSync,renameSync,realpathSync} from 'node:fs';
import path from 'node:path';
import {chromium,expect} from '@playwright/test';
import {DatabaseSync} from 'node:sqlite';
import {databaseInventory,seedLegacyRows,legacySeedRows} from './windows-update-fixtures.mjs';
import {readTools,writeTools,readMechanismEvidence,writeMechanismEvidence,proofMarkers} from './windows-update-artifacts.mjs';
import {temporaryPreferenceCleanup} from './windows-update-registration.mjs';
import {installedStartup,restartAndReconnect,aclPrelude,aclSnapshotScript,aclRestoreScript,deploymentReading} from './windows-update-startup.mjs';
import {readPackages,writePackages,nsisExecutableIdentity,nsisPayloadInventory,nsisHelperIdentity} from './windows-update-packages.mjs';
import {prepareDisposableDebugPolicy} from './windows-update-debug-policy.mjs';
if(process.platform!=='win32'||process.env.GITHUB_ACTIONS!=='true'&&process.env.SAYWEAVE_DISPOSABLE_WINDOWS!=='1')throw Error('Use an elevated disposable Windows VM; never an owner machine.');
const root=process.cwd(),id=randomUUID(),directory=path.join(root,'.local','windows-update',id),source=path.join(directory,'source');mkdirSync(source,{recursive:true});
const stableVersion=JSON.parse(readFileSync(path.join(root,'package.json'),'utf8')).version;
const acceptanceCommit=execFileSync('git',['rev-parse','HEAD'],{encoding:'utf8'}).trim(),frozenProduct=process.env.FROZEN_ACCEPTANCE_PRODUCT||null,commit=frozenProduct??acceptanceCommit;
if(frozenProduct){
 if(!/^[a-f0-9]{40}$/.test(frozenProduct))throw Error('Exact frozen acceptance product required');
 const allowed=new Set(['.github/workflows/windows-ci.yml','scripts/test-windows-update.mjs','scripts/test-windows-installed-startup.mjs','scripts/windows-update-startup.mjs','scripts/windows-update-packages.mjs','scripts/windows-update-debug-policy.mjs','tests/windows-update-startup.test.ts','tests/release-validation.test.ts','docs/testing.md','docs/release.md']);
 const changed=execFileSync('git',['diff','--name-only',frozenProduct,acceptanceCommit],{encoding:'utf8'}).trim().split('\n').filter(Boolean);
 if(changed.some(file=>!allowed.has(file))||execFileSync('git',['status','--porcelain','--untracked-files=no'],{encoding:'utf8'}).trim())throw Error('Frozen product differs outside clean acceptance-controller scope');
}
const report={schemaVersion:1,commit,acceptanceCommit,frozenProductReuse:Boolean(frozenProduct),environment:process.env.RUNNER_NAME??'explicit disposable Windows',id,checks:[],installers:[],commands:[],passed:false};
const argument=name=>{const n=process.argv.indexOf(name);if(n<0)return null;if(!process.argv[n+1]||process.argv[n+1].startsWith('--'))throw Error(`Missing ${name} value`);return path.resolve(process.argv[n+1]);};
const toolsOnly=process.argv.includes('--tools-only'),startupOnly=process.argv.includes('--startup-only'),packagesDirectory=argument('--packages-dir'),stableInstaller=argument('--stable-installer'),evidenceDirectory=argument('--mechanism-evidence'),toolsDirectory=argument('--tools-dir');
if(frozenProduct&&!toolsDirectory&&!evidenceDirectory)throw Error('Frozen acceptance requires verified immutable tools; fallback compilation forbidden');
if(toolsOnly&&(stableInstaller||evidenceDirectory)||Boolean(stableInstaller)!==Boolean(evidenceDirectory))throw Error('Formal acceptance requires a draft installer and passed same-commit mechanism evidence');
if(startupOnly&&(toolsOnly||stableInstaller)||packagesDirectory&&(toolsOnly||stableInstaller))throw Error('Startup packages are isolated from tooling-only and formal acceptance');
const priorPackages=packagesDirectory?readPackages(packagesDirectory,commit):null;
if(frozenProduct&&!toolsOnly&&(!priorPackages?.next||stableInstaller))throw Error('Frozen acceptance requires the complete immutable test pair; no product rebuild');
report.kind=toolsOnly?'tools':stableInstaller?'formal-installer':startupOnly?'startup':'mechanism';
const log=path.join(directory,'build.log'),keyPath=path.join(directory,'fixture-signing.key'),cli=path.join(root,'node_modules','@tauri-apps','cli','tauri.js');
const delay=ms=>new Promise(resolve=>setTimeout(resolve,ms));
function checkpoint(){
 report.updatedAt=new Date().toISOString();
 for(const file of [path.join(directory,'acceptance.json'),path.join(root,'.local','windows-update-acceptance.json')]){writeFileSync(file+'.tmp',JSON.stringify(report,null,2)+'\n');renameSync(file+'.tmp',file);}
}
async function command(file,args,cwd=root,env=process.env,expectedFailure,timeoutMs=30*60_000){
 const started=Date.now(),entry={stage:report.stage,tool:path.basename(file),startedAt:new Date(started).toISOString()};report.commands.push(entry);checkpoint();
 let tail='',stdout='',stderr='';const safe=path.basename(file)==='tsf-probe.exe'||file==='powershell'&&(toolsOnly||args.includes('scripts/windows-update-metadata.ps1')||args.at(-1)?.startsWith(aclPrelude)||args.at(-1)?.startsWith("$ErrorActionPreference='Stop';$v=(Get-Item -LiteralPath "))||file==='cargo'&&(args.includes('verify-update-signature')||args[0]==='build'&&args.some(arg=>arg.includes('text-service')));
 if(safe)entry.args=args;
 const diagnostics=path.join(directory,'diagnostics');if(safe)mkdirSync(diagnostics,{recursive:true});
 const child=spawn(file,args,{cwd,env,windowsHide:true,timeout:timeoutMs,stdio:['ignore','pipe','pipe']});
 for(const [label,stream] of [['stdout',child.stdout],['stderr',child.stderr]])stream.on('data',bytes=>{writeFileSync(log,bytes,{flag:'a'});tail=(tail+bytes.toString('utf8')).slice(-16384);if(label==='stdout')stdout=(stdout+bytes.toString('utf8')).slice(-131072);else stderr=(stderr+bytes.toString('utf8')).slice(-131072);});
 const heartbeat=setInterval(()=>{entry.elapsedMs=Date.now()-started;checkpoint();console.log(`Windows update acceptance: ${entry.stage} (${Math.round(entry.elapsedMs/1000)}s)`);},60000);
 let code,spawnError;child.once('error',error=>{spawnError=error;});
 try{code=await new Promise(resolve=>child.once('close',(value,signal)=>{entry.signal=signal;resolve(value);}));}finally{clearInterval(heartbeat);entry.elapsedMs=Date.now()-started;entry.exitCode=code??null;entry.timeout=child.killed;if(spawnError)entry.spawnError=spawnError.code;if(safe){entry.stdout=stdout.slice(-8192);entry.stderr=stderr.slice(-8192);for(const [label,value] of [['stdout',stdout],['stderr',stderr]])writeFileSync(path.join(diagnostics,`${report.commands.length}-${entry.tool}.${label}.log`),value);}checkpoint();}
 if(spawnError)throw spawnError;
 if(child.killed)throw Error(`Command exceeded ${timeoutMs}ms: ${path.basename(file)}`);
 if(expectedFailure){if(code!==0&&(expectedFailure===true||tail.includes(expectedFailure)))return;throw Error('Expected command rejection was not observed');}
 if(code!==0)throw Error(`Command failed: ${file} ${args.slice(0,3).join(' ')} (${code}); ${safe?stderr.slice(-4096):'private fixture log retained'}`);
 return {stdout,stderr};
}
function stage(value){report.stage=value;checkpoint();console.log(`Windows update acceptance: ${value}`);}
let server,browser,app,thumbprint,ownedProcess,lastEndpoint,serverMode='valid',packageBytes,signature,deniedDirectory,originalAcl,installerProcess;
let installerDescendants=[];
let installerTreeConfirmed=false;
let installerLaunchAttempted=false;
let credentialRequests=0;
let tools,legacySeed,temporaryRegistration,temporaryMachineRegistration,temporaryMachineFiles,unknownPreference,debugPolicy,signatureProject,certificatePrivateKey;
const payloadProofCache=new Map();
async function candidatePayload(installer){
 const sha=createHash('sha256').update(readFileSync(installer)).digest('hex');
 if(payloadProofCache.has(sha))return payloadProofCache.get(sha);
 const extractor=path.join(process.env.ProgramFiles,'7-Zip','7z.exe');
 if(!existsSync(extractor))throw Error('Disposable runner lacks the candidate payload extractor');
 const inventory=nsisPayloadInventory((await command(extractor,['l','-slt',installer])).stdout);
 const output=path.join(directory,'checked-payload',sha);
 await command(extractor,['x',installer,`-o${output}`,'sayweave-installer-helper.exe','$PLUGINSDIR\\sayweave-maintenance.exe','-y']);
 const helper=nsisHelperIdentity(readFileSync(path.join(output,'sayweave-installer-helper.exe')),readFileSync(path.join(output,'$PLUGINSDIR','sayweave-maintenance.exe')));
 const proof={installerSha256:sha,inventory,...helper};(report.candidatePayloads??=[]).push(proof);payloadProofCache.set(sha,proof);checkpoint();return proof;
}
async function cleanup(label,action){
 try{await action();return true;}catch(error){report.passed=false;process.exitCode=1;(report.cleanupErrors??=[]).push({stage:label,error:String(error?.message??error?.code??error).slice(0,1024)});return false;}
}
const debugPort=9291,identifier=priorPackages?.manifest.identifier??`local.sayweave.acceptance.${id}`,product=priorPackages?.manifest.product??`Sayweave 验收 ${id}`,testData=path.join(process.env.APPDATA,identifier,'data'),testExe=path.join(process.env.LOCALAPPDATA,product,'sayweave.exe');
const invoke=(page,command,args={})=>page.evaluate(({command,args})=>window.__TAURI_INTERNALS__.invoke(command,args),{command,args});
function processes(exe){
 const literal=exe.replaceAll("'","''");
 return JSON.parse(execFileSync('powershell',['-NoProfile','-Command',`[Console]::OutputEncoding=[System.Text.UTF8Encoding]::new($false);$items=@(Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath -eq '${literal}' } | Select-Object ProcessId,@{Name='CreationDate';Expression={$_.CreationDate.ToUniversalTime().ToString('o')}},ExecutablePath);ConvertTo-Json -InputObject $items -Compress`],{encoding:'utf8',timeout:10000}));
}
function captureInstallerTree(first=false){
 const all=JSON.parse(execFileSync('powershell',['-NoProfile','-Command',"[Console]::OutputEncoding=[System.Text.UTF8Encoding]::new($false);$all=@(Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId,@{Name='CreationDate';Expression={$_.CreationDate.ToUniversalTime().ToString('o')}},ExecutablePath);ConvertTo-Json -InputObject $all -Compress"],{encoding:'utf8',timeout:10000}));
 const same=(a,b)=>a.ProcessId===b.ProcessId&&a.CreationDate===b.CreationDate&&a.ExecutablePath===b.ExecutablePath;
 if(first&&!all.some(p=>same(p,installerProcess)))throw Error('Installer root ended before its first verified ownership snapshot');
 const tree=[...installerDescendants];if(installerProcess&&!tree.some(p=>same(p,installerProcess)))tree.push(installerProcess);
 for(let n=0;n<tree.length;n++){
  const parent=tree[n];
  if(all.some(p=>p.ProcessId===parent.ProcessId&&!same(p,parent)))throw Error('Installer parent PID reused during ownership tracking');
  for(const child of all.filter(p=>p.ParentProcessId===parent.ProcessId&&p.CreationDate>=parent.CreationDate)){
   if(!Number.isInteger(child.ProcessId)||!child.ExecutablePath)throw Error('Installer descendant identity unavailable');
   if(!tree.some(p=>same(p,child)))tree.push(child);
  }
 }
 if(tree.length>64)throw Error('Owned installer tree exceeded its bounded identity inventory');
 installerDescendants=tree;if(first)installerTreeConfirmed=true;
 const snapshot={root:installerProcess??report.installerOwnership?.first?.root,members:tree,at:new Date().toISOString()};report.installerOwnership??={};report.installerOwnership[first?'first':'last']=snapshot;checkpoint();return tree;
}
async function connect(exe,launch=true,allowBroken=false){
 if(!debugPolicy){debugPolicy=prepareDisposableDebugPolicy(debugPort);report.debugPolicyLease=debugPolicy.receipt;checkpoint();debugPolicy.install();checkpoint();}
 const result=await installedStartup({exe,dataRoot:exe===testExe?testData:path.join(process.env.APPDATA,'local.sayweave.skeleton','data'),directory:path.join(directory,'diagnostics'),port:debugPort,launch,previous:ownedProcess,record:report.startups??(report.startups=[]),persist:checkpoint,onOwned:value=>{ownedProcess=value;},onSpawn:value=>{app=value;},allowMissingPageMarker:allowBroken});
 const endpoint=result.endpoint;if(!launch&&endpoint.webSocketDebuggerUrl===lastEndpoint)throw Error('Old debug endpoint cannot confirm restart');lastEndpoint=endpoint.webSocketDebuggerUrl;
 try{
  browser=await chromium.connectOverCDP(endpoint.webSocketDebuggerUrl,{timeout:10000});let page;for(let n=0;n<100;n++){page=browser.contexts().flatMap(c=>c.pages()).find(p=>p.url().includes('tauri')&&!p.url().includes('surface=overlay'));if(page)break;await delay(100);}if(!page)throw Error('Main WebView missing');await page.waitForFunction(()=>window.__TAURI_INTERNALS__?.invoke,{},{timeout:10000});if(!allowBroken)await expect(page.locator('aside')).toBeVisible();result.entry.rendered=!allowBroken;result.entry.phase='rendererReady';checkpoint();return page;
 }catch(error){result.entry.passed=false;result.entry.phase='renderer';result.entry.error=String(error.message).slice(0,2048);checkpoint();throw error;}
}
async function close(page,acceptance=true){
 if(acceptance)await invoke(page,'acceptance_update_exit').catch(()=>{});if(browser){await browser.close().catch(()=>{});browser=null;}
 if(ownedProcess){
  if(acceptance)for(let n=0;n<20;n++){if(!processes(ownedProcess.ExecutablePath).some(p=>p.CreationDate===ownedProcess.CreationDate))break;await delay(250);}
  const remaining=processes(ownedProcess.ExecutablePath).find(p=>p.CreationDate===ownedProcess.CreationDate);
  if(remaining){if(acceptance)throw Error('Native exit did not complete');execFileSync('taskkill',['/PID',String(remaining.ProcessId),'/T','/F'],{stdio:'ignore'});}
 }app=null;ownedProcess=null;await delay(600);
}
async function restartCurrent(page){
 return restartAndReconnect({original:ownedProcess,invoke:()=>invoke(page,'update_restart'),disconnect:async()=>{await browser.close().catch(()=>{});browser=null;},inspect:()=>processes(testExe),reconnect:()=>connect(testExe,false),current:()=>ownedProcess,record:report.restarts??(report.restarts=[]),persist:checkpoint});
}
async function state(page,wanted){await expect.poll(async()=>(await invoke(page,'update_snapshot')).state,{timeout:60000}).toBe(wanted);return invoke(page,'update_snapshot');}
async function install(installer){await candidatePayload(installer);try{await command(installer,['/S']);if(!existsSync(testExe))throw Error('NSIS current-user path differs');}finally{(report.deployments??=[]).push({stage:report.stage,installerSha256:createHash('sha256').update(readFileSync(installer)).digest('hex'),records:deploymentReading(path.dirname(testExe))});checkpoint();}}
const data=page=>invoke(page,'acceptance_update_data');
async function verifyPackage(file,version,publicKey){
 stage(`signature verification ${version}`);
 if(frozenProduct){
  if(!signatureProject){
   signatureProject=path.join(directory,'signature-verifier');mkdirSync(signatureProject);
   writeFileSync(path.join(signatureProject,'Cargo.toml'),'[package]\nname="frozen-acceptance-signature"\nversion="0.0.0"\nedition="2024"\n[dependencies]\nbase64="=0.22.1"\nminisign-verify="=0.2.5"\n[[bin]]\nname="verify"\npath="main.rs"\n');
   const signatureSource=path.join(source,'src-tauri/src/update_signature.rs').replaceAll('\\','/'),mainSource=path.join(source,'src-tauri/src/bin/verify_update_signature.rs').replaceAll('\\','/');
   writeFileSync(path.join(signatureProject,'main.rs'),`#[path=${JSON.stringify(signatureSource)}] pub mod update_signature;\nmod sayweave_lib {pub use crate::update_signature;}\ninclude!(${JSON.stringify(mainSource)});\n`);
  }
  await command('cargo',['run','--quiet','--manifest-path',path.join(signatureProject,'Cargo.toml'),'--',publicKey,file,`${file}.sig`,version]);
 }else await command('cargo',['run','--quiet','--manifest-path','src-tauri/Cargo.toml','--bin','verify-update-signature','--',publicKey,file,`${file}.sig`,version]);
}
function installerIdentity(file,version,channel){
 const bytes=readFileSync(file);
 return {version,channel,sha256:createHash('sha256').update(bytes).digest('hex'),sha512:createHash('sha512').update(bytes).digest('base64'),size:bytes.length};
}
function componentIdentity(){
 const module=execFileSync('powershell',['-NoProfile','-Command',"(Get-Item -LiteralPath 'Registry::HKEY_LOCAL_MACHINE\\Software\\Classes\\CLSID\\{CB473CF0-9B4C-482C-BFC2-5527D2379ACA}\\InprocServer32').GetValue('')"],{encoding:'utf8'}).trim();
 if(!path.isAbsolute(module))throw Error('Installed component path missing');
 return {module,sha256:createHash('sha256').update(readFileSync(module)).digest('hex')};
}
function quote(value){return "'"+value.replaceAll("'","''")+"'";}
async function registrationInventory(timeoutMs=30000){
 const script="$ErrorActionPreference='Stop';$items=@{};foreach($scope in @('HKEY_CURRENT_USER','HKEY_LOCAL_MACHINE')){$base='Registry::'+$scope;$com=$base+'\\Software\\Classes\\CLSID\\{CB473CF0-9B4C-482C-BFC2-5527D2379ACA}';$tip=$base+'\\Software\\Microsoft\\CTF\\TIP\\{CB473CF0-9B4C-482C-BFC2-5527D2379ACA}';$item=@{com=(Test-Path -LiteralPath $com);tip=(Test-Path -LiteralPath $tip);module=$null;owner=$null;tipEntries=@()};if(Test-Path -LiteralPath ($com+'\\InprocServer32')){$key=Get-Item -LiteralPath ($com+'\\InprocServer32');$item.module=$key.GetValue('');$item.owner=$key.GetValue('SayweaveOwner')};if($item.tip){$queue=@($tip);for($n=0;$n -lt $queue.Count;$n++){if($queue.Count -gt 64){throw 'Owned TIP registry inventory exceeded bound'};$key=Get-Item -LiteralPath $queue[$n];$values=@{};$names=@($key.GetValueNames());if($names.Count -gt 32){throw 'Owned TIP value inventory exceeded bound'};foreach($name in $names){$value=$key.GetValue($name);if(($value -isnot [string] -and $value -isnot [int]) -or ([string]$value).Length -gt 1024){throw 'Owned TIP value exceeded bound'};$values[$name]=@{kind=([string]$key.GetValueKind($name));value=$value}};$item.tipEntries+=@{path=$queue[$n].Substring($tip.Length);values=$values};foreach($child in @($key.GetSubKeyNames())){$queue+=($queue[$n]+'\\'+$child)}}};$items[$scope]=$item};ConvertTo-Json -InputObject $items -Compress -Depth 9";
 return JSON.parse((await command('powershell',['-NoProfile','-Command',script],root,process.env,undefined,timeoutMs)).stdout);
}
const registrationEmpty=info=>Object.values(info).every(item=>!item.com&&!item.tip);
async function characterizeComponent(module,sha256){
 const result=await command('powershell',['-NoProfile','-ExecutionPolicy','Bypass','-File','scripts/windows-update-metadata.ps1','-ExpectedCommit',acceptanceCommit,'-ProductCommit',commit,'-ExpectedModulePath',module,'-ExpectedComponentSha256',sha256],root,process.env,undefined,30000);
 const end=result.stdout.indexOf('Read-only metadata receipt:');if(end<0)throw Error('Metadata receipt output missing');
 const receipt=JSON.parse(result.stdout.slice(0,end).trim());(report.metadataReadings??=[]).push(receipt);checkpoint();expect(receipt.characterization.readComplete).toBe(true);expect(receipt.metadataExpected).toBe(true);return receipt;
}
async function clearMachineRegistration(){
 if(!temporaryMachineRegistration)return;
 const current=await registrationInventory(),identity=current.HKEY_LOCAL_MACHINE;
 if(registrationEmpty(current)){await awaitRegistrationState(tools.probe,false);temporaryMachineRegistration=null;return;}
 if(!identity.com||identity.owner!=='SayweaveStageA-v1'||typeof identity.module!=='string'||!temporaryMachineRegistration.paths.some(module=>moduleKey(module)===moduleKey(identity.module))||createHash('sha256').update(readFileSync(identity.module)).digest('hex')!==temporaryMachineRegistration.sha256)throw Error('Machine metadata fixture ownership changed');
 await command(tools.probe,['remove-machine'],root,process.env,undefined,30000);
 report.machineRemoval={inventory:await registrationInventory(),api:await awaitRegistrationState(tools.probe,false)};checkpoint();
 if(!registrationEmpty(report.machineRemoval.inventory))throw Error('Product machine removal left registrations');
 temporaryMachineRegistration=null;
}
async function clearMachineFiles(){
 if(!temporaryMachineFiles)return;
 if(!registrationEmpty(await registrationInventory()))throw Error('Fixture files remain registered; retaining them for diagnosis');
 await awaitRegistrationState(tools.probe,false);
 for(const module of temporaryMachineFiles.paths){if(!existsSync(module))continue;if(createHash('sha256').update(readFileSync(module)).digest('hex')!==temporaryMachineFiles.sha256)throw Error('Fixture module changed');rmSync(module);}
 for(const directory of [...temporaryMachineFiles.directories].reverse()){if(existsSync(directory))rmdirSync(directory);}
 temporaryMachineFiles=null;
}
async function clearUnknownPreference(){
 if(!unknownPreference)return;
 const {key,name}=unknownPreference;
 await command('powershell',['-NoProfile','-Command',`$ErrorActionPreference='Stop';$key=[Microsoft.Win32.Registry]::CurrentUser.OpenSubKey(${quote(key)},$true);if(-not $key){throw 'Fixture key missing'};try{if($key.GetValueNames() -contains ${quote(name)}){if($key.GetValueKind(${quote(name)}) -ne [Microsoft.Win32.RegistryValueKind]::DWord -or $key.GetValue(${quote(name)}) -ne 123){throw 'Fixture value changed'};$key.DeleteValue(${quote(name)},$true)}}finally{$key.Close()}`]);
 unknownPreference=null;
}
async function machineMetadataProof(){
 stage('owned machine metadata repair and rollback');
 if(!registrationEmpty(await registrationInventory()))throw Error('Machine metadata proof requires absent registration');
 const base=path.join(process.env.ProgramFiles,'Sayweave','TextService'),componentHash=createHash('sha256').update(readFileSync(tools.component)).digest('hex');
 if(existsSync(path.dirname(base)))throw Error('Machine metadata fixture parent already exists');
 const paths=[componentHash,createHash('sha256').update(`replacement-fixture-${id}`).digest('hex')].map(folder=>path.join(base,folder,'sayweave_text_service.dll'));
 temporaryMachineFiles={paths,sha256:componentHash,directories:[]};
 for(const directory of [path.dirname(base),base,...paths.map(module=>path.dirname(module))]){mkdirSync(directory);temporaryMachineFiles.directories.push(directory);}
 for(const module of paths)writeFileSync(module,readFileSync(tools.component),{flag:'wx'});
 temporaryMachineRegistration={paths,sha256:componentHash};
 await command(tools.probe,['register-machine',paths[0]],root,process.env,undefined,30000);
 const prepared=await registrationInventory();report.machineMetadata={syntheticPaths:true,prepared,initial:await characterizeComponent(paths[0],componentHash)};checkpoint();
 const leaf=prepared.HKEY_LOCAL_MACHINE.tipEntries.find(entry=>Object.hasOwn(entry.values,'Description'))?.path;
 if(!leaf)throw Error('Machine metadata profile missing');
 const key=`Software\\Microsoft\\CTF\\TIP\\{CB473CF0-9B4C-482C-BFC2-5527D2379ACA}${leaf}`;
 await command('powershell',['-NoProfile','-Command',`$ErrorActionPreference='Stop';$key=[Microsoft.Win32.Registry]::LocalMachine.OpenSubKey(${quote(key)},$true);if(-not $key){throw 'Fixture key missing'};try{$key.SetValue('Description','Sayweave Voice Files',[Microsoft.Win32.RegistryValueKind]::String)}finally{$key.Close()}`]);
 const malformed=await registrationInventory();report.machineMetadata.malformed=malformed;checkpoint();
 await command(tools.probe,['repair-machine-metadata',paths[1],prepared.HKEY_LOCAL_MACHINE.module,'reject-confirmation'],root,process.env,'acceptanceReplacementFailure',30000);
 expect(await registrationInventory()).toEqual(malformed);report.machineMetadata.rollbackPreserved=true;checkpoint();
 await command(tools.probe,['repair-machine-metadata',paths[1],prepared.HKEY_LOCAL_MACHINE.module],root,process.env,undefined,30000);
 const repaired=await registrationInventory(),expected=structuredClone(prepared);expected.HKEY_LOCAL_MACHINE.module=paths[1];
 expected.HKEY_LOCAL_MACHINE.tipEntries.find(entry=>Object.hasOwn(entry.values,'IconFile')).values.IconFile.value=paths[1];
 // Register stores the canonical Win32 extended path; replacement preserves the provided path.
 expect(repaired).toEqual(expected);report.machineMetadata.repaired=await characterizeComponent(paths[1],componentHash);checkpoint();
 await clearMachineRegistration();
 await clearMachineFiles();
 report.checks.push('owned machine replacement repairs malformed raw/API metadata without changing profile enablement; post-write failure restores exact prior registry; native machine removal is empty');
}
async function awaitRegistrationState(probe,registered,requireEmptyRoots=true){
 const deadline=Date.now()+10000;let state,stable=0;
 do{
  state=JSON.parse((await command(probe,['registration-state'],root,process.env,undefined,Math.max(1,deadline-Date.now()))).stdout.trim());
  let emptyRoots=true;
  if(!registered){const inventory=await registrationInventory(Math.max(1,deadline-Date.now()));report.toolRegistrationLastCleanup={inventory,profiles:state};checkpoint();emptyRoots=!requireEmptyRoots||registrationEmpty(inventory);}
  const matches=registered?state.profiles>0&&state.categories>0&&state.speechCategory:state.profiles===0&&state.categories===0&&!state.speechCategory&&emptyRoots;
  stable=matches?stable+1:0;if(stable===2&&Date.now()<=deadline)return state;
  await delay(Math.max(0,Math.min(250,deadline-Date.now())));
 }while(Date.now()<deadline);
 throw Error(`TIP registration state did not settle (${registered?'registered':'removed'}): ${JSON.stringify(state)}`);
}
async function installedRegistrationReady(probe){
 const context=report.stage;stage('installed registration readiness');const started=Date.now(),commandStart=report.commands.length;
 const api=await awaitRegistrationState(probe,true);
 const detail=JSON.parse((await command(probe,['registration-query-detail'],root,process.env,undefined,30000)).stdout.trim());
 report.installedRegistrationReadiness={context,elapsedMs:Date.now()-started,api,detail,commandIndexes:report.commands.slice(commandStart).map((_entry,index)=>commandStart+index)};checkpoint();stage(context);
}
function moduleKey(value){return (value.startsWith('\\\\?\\')?value.slice(4):value).toLowerCase();}
async function clearTemporaryRegistration(){
 if(!temporaryRegistration)return;
 const before=await registrationInventory(),user=before.HKEY_CURRENT_USER,machine=before.HKEY_LOCAL_MACHINE;
 if(user.com||machine.com){
  if(!user.com||user.owner!=='SayweaveStageA-v1'||typeof user.module!=='string'||moduleKey(user.module)!==moduleKey(temporaryRegistration.component)||machine.com)throw Error('Temporary TIP identity is unavailable or changed; no blind removal');
  temporaryRegistration.anchor=before;report.toolRegistrationRemoval={anchor:before,initial:temporaryRegistration.initial,componentSha256:temporaryRegistration.sha256};checkpoint();
  try{await command(temporaryRegistration.probe,['remove'],root,process.env,undefined,30000);}
  catch(error){
   const failure=report.nativeRemovalFailure={error:String(error.message).slice(0,4096)};checkpoint();
   try{failure.inventory=await registrationInventory();}catch(readError){failure.inventoryError=String(readError.message).slice(0,1024);}checkpoint();
   try{failure.api=JSON.parse((await command(temporaryRegistration.probe,['registration-state'],root,process.env,undefined,30000)).stdout.trim());}catch(readError){failure.apiError=String(readError.message).slice(0,1024);}checkpoint();
   throw error;
  }
  temporaryRegistration.apiRemoved=true;
 }
 const metadata=await awaitRegistrationState(temporaryRegistration.probe,false,false),remaining=await registrationInventory();
 if(!registrationEmpty(remaining)){
  if(!temporaryRegistration.apiRemoved||createHash('sha256').update(readFileSync(temporaryRegistration.component)).digest('hex')!==temporaryRegistration.sha256)throw Error('Owned TIP removal identity is unavailable');
  const paths=temporaryPreferenceCleanup({initial:temporaryRegistration.initial,prepared:temporaryRegistration.prepared,anchor:temporaryRegistration.anchor,after:remaining,component:temporaryRegistration.component,metadata});
  report.toolRegistrationPreferenceCleanup={before:remaining,paths};checkpoint();
  // Recheck every fixed-class key immediately before mutation. Delete only the
  // observed Enable value, then empty leaves with non-recursive DeleteSubKey.
  const script=String.raw`$ErrorActionPreference='Stop';$base='Software\Microsoft\CTF\TIP\{CB473CF0-9B4C-482C-BFC2-5527D2379ACA}';$root=[Microsoft.Win32.Registry]::CurrentUser;$paths=ConvertFrom-Json ${quote(JSON.stringify(paths))};foreach($scope in @('HKEY_CURRENT_USER','HKEY_LOCAL_MACHINE')){if(Test-Path -LiteralPath ('Registry::'+$scope+'\Software\Classes\CLSID\{CB473CF0-9B4C-482C-BFC2-5527D2379ACA}')){throw 'COM registration reappeared'}};if(Test-Path -LiteralPath ('Registry::HKEY_LOCAL_MACHINE\'+$base)){throw 'Machine TIP registration reappeared'};for($n=0;$n -lt $paths.Count;$n++){$key=$root.OpenSubKey($base+$paths[$n]);if(-not $key){throw 'Owned TIP preference changed'};try{if($n -eq 0){if($key.SubKeyCount -ne 0 -or $key.ValueCount -ne 1 -or $key.GetValueKind('Enable') -ne [Microsoft.Win32.RegistryValueKind]::DWord -or $key.GetValue('Enable') -ne 1){throw 'Owned Enable preference changed'}}else{$children=@($key.GetSubKeyNames());$expected=$paths[$n-1].Substring($paths[$n].Length+1);if($key.ValueCount -ne 0 -or $children.Count -ne 1 -or $children[0] -cne $expected){throw 'Owned TIP parent changed'}}}finally{$key.Close()}};$key=$root.OpenSubKey($base+$paths[0],$true);try{if(-not $key -or $key.SubKeyCount -ne 0 -or $key.ValueCount -ne 1 -or $key.GetValueKind('Enable') -ne [Microsoft.Win32.RegistryValueKind]::DWord -or $key.GetValue('Enable') -ne 1){throw 'Owned Enable preference changed'};$key.DeleteValue('Enable',$true)}finally{if($key){$key.Close()}};foreach($relative in $paths){$subkey=$base+$relative;$key=$root.OpenSubKey($subkey);if(-not $key){throw 'Owned TIP leaf changed'};try{if($key.ValueCount -ne 0 -or $key.SubKeyCount -ne 0){throw 'Owned TIP leaf is not empty'}}finally{$key.Close()};$root.DeleteSubKey($subkey,$true)};'Owned temporary TIP Enable preference removed with empty-leaf deletion'`;
  const result=await command('powershell',['-NoProfile','-Command',script],root,process.env,undefined,5000);report.toolRegistrationPreferenceCleanup.result=result.stdout.trim();checkpoint();
 }
 const profiles=await awaitRegistrationState(temporaryRegistration.probe,false),after=await registrationInventory();
 report.toolRegistration={before,after,profiles,removed:registrationEmpty(after)};checkpoint();
 if(!registrationEmpty(after))throw Error('Temporary TIP registration was not fully removed');
 report.toolRegistration={before,after,profiles,removed:true};temporaryRegistration=null;
}
async function restoreInstallAcl(){
 if(!deniedDirectory)return;
 const script=aclRestoreScript(deniedDirectory,originalAcl);
 const result=await command('powershell',['-NoProfile','-Command',script],root,process.env,undefined,30000);
 report.aclRestoration=JSON.parse(result.stdout.trim());expect(report.aclRestoration.descriptorMatches).toBe(true);
 const proof=path.join(deniedDirectory,`update-acl-restored-${id}.tmp`);writeFileSync(proof,'restored');rmSync(proof);report.aclRestoreWriteProbe=true;checkpoint();deniedDirectory=null;
}
async function legacyData(page,config){
 await expect.poll(async()=>(await invoke(page,'model_status',{retry:false})).state).toBe('ready');
 const before=credentialRequests;
 const input={name:config.name,protocol:config.protocol,baseUrl:config.baseUrl,modelId:config.modelId,usages:config.usages};
 await invoke(page,'model_test',{draft:{input,id:config.id,revision:config.revision,key:{action:'keep'}},requestId:randomUUID()});
 expect(credentialRequests).toBe(before+1);
 const models=await invoke(page,'model_overview'),dictionary=await invoke(page,'dictionary_list',{query:{kind:'term',search:'',page:1}}),history=await invoke(page,'history_usage'),profile=await invoke(page,'language_overview',{after:null}),component=await invoke(page,'text_service_status');
 expect(component.installed).toBe(true);expect(component.error).toBeNull();
 const application=profile.applications[0];
 const detail=await invoke(page,'language_profile',{scope:application.scope});
 return {models,dictionary,history,profile,detail,component:{installed:component.installed,enabled:component.enabled,error:component.error}};
}
async function ready(page){await invoke(page,'update_check',{manual:true});return state(page,'readyToInstall');}
async function nativeLayout(page,label){
 for(const [minimum,zoom,motion] of [[false,1,'no-preference'],[true,1,'reduce'],[true,1.5,'reduce']]){
  await invoke(page,'acceptance_main_size',{minimum,zoom});await page.emulateMedia({reducedMotion:motion});
  await expect.poll(()=>page.evaluate(()=>({width:innerWidth,height:innerHeight})),{timeout:5000}).toEqual({width:Math.round((minimum?960:1120)/zoom),height:Math.round((minimum?640:760)/zoom)});
  const update=await invoke(page,'update_snapshot');
  await (update.version||update.recovery?page.locator('aside button[aria-haspopup="dialog"]'):page.getByRole('button',{name:'关于',exact:true})).click();const dialog=page.getByRole('dialog');await expect(dialog).toBeVisible();
  const panel=dialog.locator('section[aria-label="软件更新"]');await expect(panel).toBeVisible();
  const samples=[];let geometry,previous,stable=0;
  try{await expect.poll(async()=>{geometry=await dialog.evaluate(el=>{const r=el.getBoundingClientRect(),button=[...document.querySelectorAll('button[aria-controls]')].find(b=>b.getAttribute('aria-controls')===el.id),anchor=button?.getBoundingClientRect();return{x:r.x,y:r.y,right:r.right,bottom:r.bottom,width:innerWidth,height:innerHeight,ratio:devicePixelRatio,anchor:anchor?{x:anchor.x,y:anchor.y,right:anchor.right,bottom:anchor.bottom}:null};});if(samples.length<32)samples.push({at:Date.now(),state:update.state,...geometry});const bounded=geometry.x>=0&&geometry.y>=0&&geometry.right<=geometry.width+1&&geometry.bottom<=geometry.height+1,unchanged=previous&&['x','y','right','bottom','width','height','ratio'].every(key=>Math.abs(geometry[key]-previous[key])<0.5);stable=bounded&&unchanged?stable+1:0;previous=geometry;return stable>=1;},{timeout:5000,intervals:[50,100,200]}).toBe(true);}finally{(report.layoutSamples??=[]).push({label,minimum,zoom,samples});checkpoint();}
  expect(geometry.x).toBeGreaterThanOrEqual(0);expect(geometry.y).toBeGreaterThanOrEqual(0);expect(geometry.right).toBeLessThanOrEqual(geometry.width+1);expect(geometry.bottom).toBeLessThanOrEqual(geometry.height+1);
  for(const button of await panel.getByRole('button').all()){await button.scrollIntoViewIfNeeded();expect(await button.evaluate(el=>{const r=el.getBoundingClientRect();return el.contains(document.elementFromPoint(r.left+r.width/2,r.top+r.height/2));})).toBe(true);}
  const button=panel.getByRole('button').first();await button.focus();await expect(button).toBeFocused();await page.keyboard.press('Tab');expect(await dialog.evaluate(el=>el.contains(document.activeElement))).toBe(true);
  await page.screenshot({path:path.join(directory,`native-ui-${label}-${minimum?'960':'1120'}-${zoom}.png`)});await page.keyboard.press('Escape');await expect(dialog).toHaveCount(0);
  report.checks.push(`native ${label} layout ${minimum?'960x640':'1120x760'} WebView zoom${zoom}, ${motion}, focus and hit testing`);
 }
 await invoke(page,'acceptance_main_size',{minimum:false,zoom:1});await page.emulateMedia({reducedMotion:'no-preference'});
}
async function build(version,channel='test'){
 if(frozenProduct)throw Error('Product build forbidden in frozen acceptance');
 stage(`building ${channel} ${version}`);
 tools=readTools(tools.directory,commit);
 const config={version,identifier:channel==='test'?identifier:'local.sayweave.skeleton',productName:channel==='test'?product:'Sayweave',bundle:{windows:{nsis:{installMode:'currentUser'}}}},file=path.join(directory,`config-${version}-${channel}.json`);writeFileSync(file,JSON.stringify(config));
 const env={...process.env,CARGO_TARGET_DIR:path.join(root,'src-tauri','target'),SAYWEAVE_BUILD_COMMIT:commit,SAYWEAVE_BUILD_CHANNEL:channel,SAYWEAVE_BUILD_VERSION:version,SAYWEAVE_TSF_PAYLOAD:tools.component,TAURI_SIGNING_PRIVATE_KEY:keyPath,TAURI_SIGNING_PRIVATE_KEY_PASSWORD:''};
 if(channel==='test'){env.SAYWEAVE_BUILD_IDENTIFIER=identifier;env.SAYWEAVE_TEST_UPDATE_URL=`https://localhost:${server.address().port}/latest.json`;env.SAYWEAVE_TEST_UPDATE_KEY=readFileSync(`${keyPath}.pub`,'utf8').trim();}else{delete env.SAYWEAVE_BUILD_IDENTIFIER;delete env.SAYWEAVE_TEST_UPDATE_URL;delete env.SAYWEAVE_TEST_UPDATE_KEY;}
 await command(process.execPath,[path.join(source,'scripts','desktop.mjs'),'package','--config',file,...(channel==='test'?['--features','desktop-acceptance']:[])],source,env);
 readTools(tools.directory,commit);
 const installer=path.join(env.CARGO_TARGET_DIR,'release','bundle','nsis',`${config.productName}_${version}_x64-setup.exe`),copy=path.join(directory,`${channel}-${version}-setup.exe`);writeFileSync(copy,readFileSync(installer));writeFileSync(`${copy}.sig`,readFileSync(`${installer}.sig`));
 await verifyPackage(copy,version,readFileSync(`${keyPath}.pub`,'utf8').trim());
 stage(`signed NSIS payload identity ${version}`);
 const extracted=path.join(directory,`payload-${channel}-${version}`),sevenZip=path.join(process.env.ProgramFiles,'7-Zip','7z.exe');
 if(!existsSync(sevenZip))throw Error('Disposable runner lacks the static signed-payload extractor');
 const inventory=nsisPayloadInventory((await command(sevenZip,['l','-slt',copy])).stdout);
 await command(sevenZip,['x',copy,`-o${extracted}`,'sayweave.exe','sayweave-installer-helper.exe','$PLUGINSDIR\\sayweave-maintenance.exe','-y']);
 const helper=nsisHelperIdentity(readFileSync(path.join(extracted,'sayweave-installer-helper.exe')),readFileSync(path.join(extracted,'$PLUGINSDIR','sayweave-maintenance.exe')));
 nsisHelperIdentity(readFileSync(path.join(extracted,'sayweave-installer-helper.exe')),readFileSync(path.join(extracted,'$PLUGINSDIR','sayweave-maintenance.exe')),readFileSync(path.join(source,'.local','installer-helper',helper.helperSha256,'sayweave-installer-helper.exe')));
 const binary=nsisExecutableIdentity(readFileSync(path.join(env.CARGO_TARGET_DIR,'release','nsis','x64','installer.nsi'),'utf8'),path.join(env.CARGO_TARGET_DIR,'release','sayweave.exe'),readFileSync(path.join(env.CARGO_TARGET_DIR,'release','sayweave.exe')),readFileSync(path.join(extracted,'sayweave.exe')));
 const identity=installerIdentity(copy,version,channel);payloadProofCache.set(identity.sha256,{installerSha256:identity.sha256,inventory,...helper});
 report.installers.push({...identity,...binary,...helper,inventory});checkpoint();return copy;
}
try{
 stage('source and build isolation');
 if(execFileSync('git',['status','--porcelain','--untracked-files=no'],{encoding:'utf8'}).trim())throw Error('Acceptance requires a clean tracked source tree');
 report.compiler={rustc:execFileSync('rustc',['--version'],{encoding:'utf8'}).trim(),cargo:execFileSync('cargo',['--version'],{encoding:'utf8'}).trim()};
 await command('powershell',['-NoProfile','-Command',"$identity=[Security.Principal.WindowsIdentity]::GetCurrent();$principal=New-Object Security.Principal.WindowsPrincipal($identity);if(-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)){throw 'Disposable VM must be elevated; no interactive elevation is attempted'}"],root,process.env,undefined,30000);
 execFileSync('git',['archive',commit,'--format=tar','-o',path.join(directory,'source.tar')]);execFileSync('tar',['-xf',path.join(directory,'source.tar'),'-C',source]);symlinkSync(path.join(root,'node_modules'),path.join(source,'node_modules'),'junction');
 const prior=evidenceDirectory?readMechanismEvidence(evidenceDirectory,commit):null;
 if(prior){tools=prior.tools;legacySeed=prior.seed;report.mechanismReceipt={commit:prior.receipt.commit,passed:prior.receipt.passed};}
 else if(toolsDirectory)tools=readTools(toolsDirectory,commit);
 else{
  const env={...process.env};delete env.CARGO_TARGET_DIR;
  const componentTarget=path.join(directory,'component-target'),probeTarget=path.join(directory,'probe-target'),manifest=path.join(source,'src-tauri/text-service/Cargo.toml');
  stage('production component tooling');await command('cargo',['build','--manifest-path',manifest,'--release','--lib','--target-dir',componentTarget],source,env);
  stage('acceptance probe tooling');await command('cargo',['build','--manifest-path',manifest,'--release','--features','acceptance','--bin','tsf-probe','--target-dir',probeTarget],source,env);
  tools={component:path.join(componentTarget,'release/sayweave_text_service.dll'),probe:path.join(probeTarget,'release/tsf-probe.exe')};
 }
 stage('short production DLL tooling proof');
 const capability=await command(tools.probe,['capabilities']);const capabilities=JSON.parse(capability.stdout.trim());expect(capabilities).toEqual({acceptance:true,installedFixture:true,protocol:8});
 stage('temporary production TIP preparation');
 const initialRegistration=await registrationInventory(),initialProfiles=await command(tools.probe,['registration-state'],root,process.env,undefined,30000);
 report.toolRegistrationInitial=initialRegistration;checkpoint();
 expect(JSON.parse(initialProfiles.stdout.trim())).toEqual({profiles:0,categories:0,speechCategory:false});
 if(!registrationEmpty(initialRegistration))throw Error('Short tooling requires an empty owned TIP registration');
 temporaryRegistration={probe:tools.probe,component:tools.component,initial:initialRegistration,sha256:createHash('sha256').update(readFileSync(tools.component)).digest('hex')};
 await command(tools.probe,['register',tools.component],root,process.env,undefined,30000);
 const prepared=await awaitRegistrationState(tools.probe,true);report.toolRegistrationPrepared=prepared;
 const preparedIdentity=await registrationInventory();report.toolRegistrationIdentity=preparedIdentity;
 const preparedUser=preparedIdentity.HKEY_CURRENT_USER;
 if(!preparedUser.com||preparedUser.owner!=='SayweaveStageA-v1'||typeof preparedUser.module!=='string'||moduleKey(preparedUser.module)!==moduleKey(tools.component)||preparedIdentity.HKEY_LOCAL_MACHINE.com)throw Error('Prepared TIP module identity differs from the verified production DLL');
 temporaryRegistration.prepared=preparedIdentity;
 stage('unknown user preference refuses native removal');
 const userLeaf=preparedUser.tipEntries.find(entry=>Object.hasOwn(entry.values,'Enable'))?.path;
 if(!userLeaf)throw Error('Prepared user preference missing');
 const userKey=`Software\\Microsoft\\CTF\\TIP\\{CB473CF0-9B4C-482C-BFC2-5527D2379ACA}${userLeaf}`,unknownName=`Acceptance-${id}`;
 unknownPreference={key:userKey,name:unknownName};
 await command('powershell',['-NoProfile','-Command',`$ErrorActionPreference='Stop';$key=[Microsoft.Win32.Registry]::CurrentUser.OpenSubKey(${quote(userKey)},$true);if(-not $key){throw 'Fixture key missing'};try{if($key.GetValueNames() -contains ${quote(unknownName)}){throw 'Fixture value exists'};$key.SetValue(${quote(unknownName)},123,[Microsoft.Win32.RegistryValueKind]::DWord)}finally{$key.Close()}`]);
 const unknown=await registrationInventory();await command(tools.probe,['remove'],root,process.env,'preferenceIdentityChanged',30000);expect(await registrationInventory()).toEqual(unknown);
 await clearUnknownPreference();
 report.checks.push('unknown preference rejects product removal with exact registry and COM anchor retained');
 stage('short production DLL metadata proof');
 report.componentMetadata=await characterizeComponent(preparedUser.module,temporaryRegistration.sha256);checkpoint();
 stage('short production DLL actual text');
 const smoke=await command(tools.probe,['installed-fixture',tools.component],root,process.env,undefined,60000);
 for(const marker of proofMarkers)expect(smoke.stdout).toContain(marker);
 if(process.env.SAYWEAVE_REGISTRATION_DIAGNOSE==='1'){
  stage('same-binary TSF removal diagnosis');
  await command(tools.probe,['registration-removal-diagnosis',tools.component],root,process.env,undefined,30000);
  throw Error('Focused TSF diagnosis complete; deliberately blocked package and acceptance progression');
 }
 stage('temporary production TIP cleanup');await clearTemporaryRegistration();
 if(report.toolRegistrationPreferenceCleanup)throw Error('Product cleanup cannot rely on a test preference deletion fallback');
 report.normalRegistrationRemoval=report.toolRegistration;
 report.checks.push('normal native removal clears exact owned registration and preferences without test deletion');
 stage('disabled profile native removal');
 const disabledInitial=await registrationInventory();
 temporaryRegistration={probe:tools.probe,component:tools.component,initial:disabledInitial,sha256:createHash('sha256').update(readFileSync(tools.component)).digest('hex')};
 await command(tools.probe,['register',tools.component],root,process.env,undefined,30000);
 await awaitRegistrationState(tools.probe,true);
 temporaryRegistration.prepared=await registrationInventory();
 await command(tools.probe,['disable-registered-profile'],root,process.env,undefined,30000);
 const disabled=await registrationInventory();report.disabledRegistration=disabled;checkpoint();
 expect(disabled.HKEY_CURRENT_USER.tipEntries.find(entry=>Object.hasOwn(entry.values,'Enable'))?.values.Enable).toEqual({kind:'DWord',value:0});
 expect(disabled.HKEY_CURRENT_USER.com).toBe(true);
 await clearTemporaryRegistration();
 if(report.toolRegistrationPreferenceCleanup)throw Error('Disabled native removal cannot rely on test deletion');
 report.disabledRegistrationRemoval=report.toolRegistration;
 report.checks.push('pre-disabled Enable DWORD0 native removal clears profiles/categories/roots and COM');
 stage('native registration failure rollback');
 temporaryRegistration={probe:tools.probe,component:tools.component,initial:await registrationInventory(),sha256:createHash('sha256').update(readFileSync(tools.component)).digest('hex')};
 await command(tools.probe,['registration-rollback',tools.component],root,process.env,'acceptanceRegistrationFailure',30000);
 expect(registrationEmpty(await registrationInventory())).toBe(true);await awaitRegistrationState(tools.probe,false);
 await clearTemporaryRegistration();
 report.checks.push('failure after profile enablement rolls back native registration, user preference and COM');
 await machineMetadataProof();
 const proof=tools.manifest?.proof??{...report.compiler,capabilities,markers:proofMarkers,registrationClean:true};
 tools=writeTools(path.join(root,'.local/windows-update-tools'),commit,readFileSync(tools.component),readFileSync(tools.probe),proof);report.tools=tools.manifest;
 if(priorPackages&&priorPackages.manifest.componentSha256!==tools.manifest.artifacts.component.sha256)throw Error('Reusable package component hash differs');
 report.checks.push('separate production DLL/acceptance probe; capabilities, actual protocol/control text, absent fault export and final COM unload passed');
 if(toolsOnly){report.passed=true;}
 else{
 if(!stableInstaller&&!frozenProduct){stage('stable feature isolation');await command('cargo',['check','--manifest-path','src-tauri/Cargo.toml','--features','desktop-acceptance'],root,{...process.env,SAYWEAVE_BUILD_CHANNEL:'stable'},'acceptance feature in stable build');report.checks.push('actual stable build rejects the desktop-acceptance feature');}
 const certRoot=frozenProduct?path.join(directory,'tls'):path.join(root,'.local','data-acceptance',id,'data'),certificateFile=path.join(certRoot,'fixture-ca.der');
 certificatePrivateKey=path.join(certRoot,'fixture-key.pem');stage('HTTPS certificate fixture');
 if(frozenProduct){
  mkdirSync(certRoot,{recursive:true});
  await command('pwsh',['-NoProfile','-Command',`$ErrorActionPreference='Stop';$rsa=[Security.Cryptography.RSA]::Create(2048);$request=[Security.Cryptography.X509Certificates.CertificateRequest]::new('CN=localhost',$rsa,[Security.Cryptography.HashAlgorithmName]::SHA256,[Security.Cryptography.RSASignaturePadding]::Pkcs1);$san=[Security.Cryptography.X509Certificates.SubjectAlternativeNameBuilder]::new();$san.AddDnsName('localhost');$request.CertificateExtensions.Add($san.Build());$request.CertificateExtensions.Add([Security.Cryptography.X509Certificates.X509BasicConstraintsExtension]::new($true,$false,0,$true));$usage=[Security.Cryptography.X509Certificates.X509KeyUsageFlags]::DigitalSignature -bor [Security.Cryptography.X509Certificates.X509KeyUsageFlags]::KeyEncipherment -bor [Security.Cryptography.X509Certificates.X509KeyUsageFlags]::KeyCertSign;$request.CertificateExtensions.Add([Security.Cryptography.X509Certificates.X509KeyUsageExtension]::new($usage,$true));$cert=$request.CreateSelfSigned([DateTimeOffset]::UtcNow.AddMinutes(-5),[DateTimeOffset]::UtcNow.AddDays(1));try{$encoding=[Text.UTF8Encoding]::new($false);[IO.File]::WriteAllBytes(${quote(certificateFile)},$cert.RawData);[IO.File]::WriteAllText(${quote(path.join(certRoot,'fixture-ca.pem'))},$cert.ExportCertificatePem(),$encoding);[IO.File]::WriteAllText(${quote(certificatePrivateKey)},$rsa.ExportPkcs8PrivateKeyPem(),$encoding)}finally{$cert.Dispose();$rsa.Dispose()}`],root,process.env,undefined,60000);
 }else await command('cargo',['test','--manifest-path','src-tauri/Cargo.toml','model_desktop_fixture','--','--ignored'],root,{...process.env,SAYWEAVE_DATA_ACCEPTANCE_ID:id});
 thumbprint=execFileSync('powershell',['-NoProfile','-Command',`(New-Object System.Security.Cryptography.X509Certificates.X509Certificate2(${quote(certificateFile)})).Thumbprint`],{encoding:'utf8',timeout:30000}).trim();
 stage('isolated HTTPS trust import');
 await command('powershell',['-NoProfile','-Command',`$ErrorActionPreference='Stop';$cert=New-Object System.Security.Cryptography.X509Certificates.X509Certificate2(${quote(certificateFile)});$store=New-Object System.Security.Cryptography.X509Certificates.X509Store('Root','LocalMachine');try{$store.Open('ReadWrite');$store.Add($cert);if($store.Certificates.Find('FindByThumbprint',${quote(thumbprint)},$false).Count -ne 1){throw 'Isolated root import not confirmed'}}finally{$store.Close();$cert.Dispose()}`],root,process.env,undefined,60000);
 report.trust={store:'LocalMachine/Root',thumbprint};report.checks.push('disposable elevated VM root imported without CurrentUser protected-root interaction; exact thumbprint confirmed');
 if(!stableInstaller&&!priorPackages){stage('temporary signature key');await command(process.execPath,[cli,'signer','generate','--ci','--write-keys',keyPath],root,process.env,undefined,60000);}
 server=createServer({cert:readFileSync(path.join(certRoot,'fixture-ca.pem')),key:readFileSync(path.join(certRoot,'fixture-key.pem'))},async(req,res)=>{
  if(req.url==='/v1/chat/completions'){
   let size=0;for await(const chunk of req){size+=chunk.length;if(size>8192){res.destroy();return;}}
   if(req.headers.authorization!=='Bearer isolated-update-test-key'){res.writeHead(401);res.end();return;}
   credentialRequests++;res.setHeader('Content-Type','application/json');res.end(JSON.stringify({choices:[{message:{content:'OK'}}]}));return;
  }
  if(req.url==='/latest.json'){res.end(JSON.stringify({version:serverMode==='wrong-version'?'0.1.702':'0.1.701',notes:'合成验收版本，保留原有资料。',platforms:{'windows-x86_64':{url:`https://localhost:${server.address().port}/package.exe`,signature:serverMode==='bad-signature'?Buffer.from('invalid signature').toString('base64'):signature,installerSize:packageBytes?.length??1,installerSha512:createHash('sha512').update(packageBytes??Buffer.from('empty')).digest('base64')}}}));return;}
  if(req.url!=='/package.exe'){res.writeHead(404);res.end();return;}res.setHeader('Content-Length',packageBytes.length);
  if(serverMode==='interrupted'){res.write(packageBytes.subarray(0,1024));setTimeout(()=>res.destroy(),100);return;}
  if(serverMode==='slow'){for(let offset=0;offset<packageBytes.length&&!res.destroyed;offset+=65536){res.write(packageBytes.subarray(offset,offset+65536));await delay(50);}res.end();return;}res.end(packageBytes);
 });await new Promise((resolve,reject)=>{server.once('error',reject);server.listen(priorPackages?.manifest.port??0,'127.0.0.1',resolve);});
 let page;
 const probe=tools.probe;
 if(!stableInstaller){
 const context=priorPackages?.manifest??{identifier,product,port:server.address().port,url:`https://localhost:${server.address().port}/latest.json`,publicKey:readFileSync(`${keyPath}.pub`,'utf8').trim(),componentSha256:tools.manifest.artifacts.component.sha256};
 const packageOutput=path.join(root,'.local/windows-update-packages');
 const publishPackages=()=>{writePackages(packageOutput,commit,context,packageEntries,report.startupProof??null);report.packages=readPackages(packageOutput,commit).manifest;checkpoint();};
 const old=priorPackages?.old??await build('0.1.700');
 const packageEntries=priorPackages?{...priorPackages.entries}:{old:{installer:old,executableSha256:report.installers.at(-1).executableSha256}};publishPackages();
 await verifyPackage(old,'0.1.700',context.publicKey);
 stage('installed startup short verification');await install(old);
 const installedComponent=componentIdentity();report.componentBefore=installedComponent;expect(installedComponent.sha256).toBe(tools.manifest.artifacts.component.sha256);
 await installedRegistrationReady(probe);await command(probe,['installed-fixture',installedComponent.module]);
 page=await connect(testExe);const buildInfo=(await invoke(page,'update_snapshot')).build;expect(buildInfo).toMatchObject({version:'0.1.700',channel:'test',commit});const installedBytes=readFileSync(testExe);expect(createHash('sha256').update(installedBytes).digest('hex')).toBe(packageEntries.old.executableSha256);for(const value of [commit,identifier,context.url,context.publicKey])if(!installedBytes.includes(Buffer.from(value)))throw Error('Frozen build context absent in installed executable');
 report.startupProof={passed:true,build:buildInfo,executableSha256:packageEntries.old.executableSha256,installerSha256:installerIdentity(old,'0.1.700','test').sha256};await close(page);publishPackages();console.log('Windows installed startup short proof passed: exact process/window/page/CDP/native renderer/build identity.');
 if(priorPackages&&!priorPackages.next&&!startupOnly)throw Error('Reusable package set has no target package; no implicit rebuild');
 const next=priorPackages?.next??(!priorPackages?await build('0.1.701'):null);
 if(next&&!packageEntries.next){packageEntries.next={installer:next,executableSha256:report.installers.at(-1).executableSha256};publishPackages();}
 if(next)await verifyPackage(next,'0.1.701',context.publicKey);
 if(startupOnly){report.checks.push('actual installed startup short proof; no update mechanism or formal package acceptance');report.passed=true;}
 else{
 packageBytes=readFileSync(next);signature=readFileSync(`${next}.sig`,'utf8').trim();
 stage('real NSIS manual replacement');page=await connect(testExe);const seeded=await invoke(page,'acceptance_update_seed');expect(seeded.credentialMatches).toBe(true);expect(seeded.history.count).toBe(10);expect(seeded.profile.preferenceCount).toBe(1);expect(seeded.schema).toBe(9);expect(seeded.component.installed).toBe(true);expect(seeded.component.updateAvailable).toBe(false);expect(seeded.component.error).toBeNull();
 await invoke(page,'text_service_action',{action:'disable'});const disabledData=await data(page);expect(disabledData.component.enabled).toBe(false);
 await close(page);const seedDb=new DatabaseSync(path.join(testData,'dictionary.sqlite'),{readOnly:true});legacySeed=legacySeedRows(seedDb);seedDb.close();
 stage('installer program-copy failure rollback');const beforeExecutable=createHash('sha256').update(readFileSync(testExe)).digest('hex');
 await candidatePayload(next);await command(next,['/S'],root,{...process.env,SAYWEAVE_INSTALL_ACCEPTANCE_FAULT:'after-program'},true);
 const receiptFolder=createHash('sha256').update(realpathSync.native(path.dirname(testExe)).replace(/^\\\\\?\\/,'').replace(/\\$/,'').toLowerCase()).digest('hex');
 const rollbackReceipt=JSON.parse(readFileSync(path.join(process.env.LOCALAPPDATA,'SayweaveInstallTransactions',receiptFolder,'receipt.json'),'utf8'));
 expect(rollbackReceipt).toMatchObject({phase:'rolledBack',lastFailure:{phase:'programInstalled',code:'acceptanceInstallerProgramFailure'},target:{version:'0.1.701',commit,mainHash:packageEntries.next.executableSha256},installer:{imageHash:createHash('sha256').update(readFileSync(next)).digest('hex')}});
 expect(rollbackReceipt.transaction).toMatch(/^[a-f0-9-]{36}$/);report.programCopyRollback={transaction:rollbackReceipt.transaction,phase:rollbackReceipt.lastFailure.phase,code:rollbackReceipt.lastFailure.code,installerSha256:rollbackReceipt.installer.imageHash,executableSha256:rollbackReceipt.target.mainHash};checkpoint();
 expect(createHash('sha256').update(readFileSync(testExe)).digest('hex')).toBe(beforeExecutable);expect(componentIdentity()).toEqual(installedComponent);
 page=await connect(testExe);expect(await data(page)).toEqual(disabledData);await close(page);report.checks.push('necessary component commit failure after program copy restores exact old EXE, component and disabled user choice with data/credentials');
 stage('manual overlay with disabled choice');await install(next);page=await connect(testExe);expect((await invoke(page,'update_snapshot')).build.version).toBe('0.1.701');expect(await data(page)).toEqual(disabledData);expect((await invoke(page,'text_service_status')).deploymentState).toBe('ready');
 await invoke(page,'text_service_action',{action:'enable'});expect(await data(page)).toEqual(seeded);await close(page);report.checks.push('real NSIS overlay automatically maintains component, preserves disabled choice, credential value, schema9, history, dictionary and profile');
 stage('real HTTPS failures and cancellation');await install(old);page=await connect(testExe);
 for(const mode of ['interrupted','bad-signature','wrong-version']){serverMode=mode;await invoke(page,'update_check',{manual:true});await state(page,'failed');expect(await data(page)).toEqual(seeded);report.checks.push(`real HTTPS ${mode} rejected; native app/data remain usable`);}
 await nativeLayout(page,'failure');
 serverMode='slow';await invoke(page,'update_check',{manual:true});await state(page,'downloading');await invoke(page,'update_cancel');await delay(500);expect((await invoke(page,'update_snapshot')).state).toBe('idle');report.checks.push('real streaming cancellation rejects late completion');serverMode='valid';let candidate=await ready(page);
 await nativeLayout(page,'ready');stage('preparation, shutdown and launch recovery');await invoke(page,'acceptance_update_hold',{kind:'mutation'});await invoke(page,'update_install',{taskId:candidate.taskId,revision:candidate.revision});candidate=await state(page,'readyToInstall');expect(candidate.failureStage).toBe('prepareTimeout');await delay(2500);expect(await data(page)).toEqual(seeded);report.checks.push('admitted mutation exceeds preparation deadline; fence releases and data remains usable');
 await invoke(page,'acceptance_update_hold',{kind:'stop'});await invoke(page,'update_install',{taskId:candidate.taskId,revision:candidate.revision});await state(page,'recovery');report.checks.push('held service stop lock exceeds five seconds; no installation; recovery displayed');
 page=await restartCurrent(page);candidate=await state(page,'readyToInstall');expect((await invoke(page,'update_snapshot')).build.version).toBe('0.1.700');expect(await data(page)).toEqual(seeded);report.checks.push('restart of old binary is not update success');
 const cache=path.join(testData,'update-cache','candidate.exe'),user=execFileSync('whoami',[],{encoding:'utf8'}).trim();await command('icacls',[cache,'/deny',`${user}:(X)`]);await invoke(page,'update_install',{taskId:candidate.taskId,revision:candidate.revision});await state(page,'recovery');await command('icacls',[cache,'/remove:d',user]);report.checks.push('real Windows execute denial produces launch recovery before process exit');
 stage('real launched installer interruption');page=await restartCurrent(page);candidate=await state(page,'readyToInstall');
 const oldExecutableHash=createHash('sha256').update(readFileSync(testExe)).digest('hex');
 const installDirectory=path.dirname(testExe),acl=await command('powershell',['-NoProfile','-Command',aclSnapshotScript(installDirectory)],root,process.env,undefined,30000);
 originalAcl=JSON.parse(acl.stdout.trim());if(!originalAcl.binary)throw Error('Original installation ACL is empty');aclRestoreScript(installDirectory,originalAcl);report.originalInstallAcl=originalAcl;deniedDirectory=installDirectory;
 await command('icacls',[deniedDirectory,'/deny',`${user}:(OI)(CI)(W,D)`]);
 let denied=false;try{writeFileSync(path.join(deniedDirectory,'must-not-create.tmp'),'forbidden');}catch{denied=true;}expect(denied).toBe(true);
 installerLaunchAttempted=true;
 const originalProcess=ownedProcess,installing=invoke(page,'update_install',{taskId:candidate.taskId,revision:candidate.revision}).catch(()=>{});
 await expect.poll(()=>{installerProcess=processes(cache)[0];if(installerProcess)captureInstallerTree(true);return Boolean(installerProcess);},{timeout:15000,intervals:[100]}).toBe(true);
 await expect.poll(()=>{captureInstallerTree();return processes(testExe).some(p=>p.CreationDate===originalProcess.CreationDate);},{timeout:15000,intervals:[100]}).toBe(false);
 const launched=installerProcess,descendants=captureInstallerTree();
 for(const owned of descendants){const remaining=processes(owned.ExecutablePath).find(p=>p.ProcessId===owned.ProcessId&&p.CreationDate===owned.CreationDate);if(remaining)execFileSync('taskkill',['/PID',String(remaining.ProcessId),'/T','/F'],{stdio:'ignore'});}
 await expect.poll(()=>descendants.some(p=>processes(p.ExecutablePath).some(current=>current.ProcessId===p.ProcessId&&current.CreationDate===p.CreationDate)),{timeout:10000}).toBe(false);installerProcess=null;
 await installing;await browser.close().catch(()=>{});browser=null;ownedProcess=null;
 await restoreInstallAcl();
 expect(createHash('sha256').update(readFileSync(testExe)).digest('hex')).toBe(oldExecutableHash);
 const journal=JSON.parse(readFileSync(path.join(testData,'update-cache','transaction.json'),'utf8'));expect(['launching','launched']).toContain(journal.stage);
 page=await connect(testExe);candidate=await state(page,'readyToInstall');expect((await invoke(page,'update_snapshot')).build.version).toBe('0.1.700');expect(candidate.error).toContain('上次更新未完成');expect(await data(page)).toEqual(seeded);
 for(let n=0;n<21;n++){expect(processes(cache)).toHaveLength(0);await delay(1000);}expect((await invoke(page,'update_snapshot')).state).toBe('readyToInstall');
 report.interruptedInstaller={...installerIdentity(cache,'0.1.701','test'),process:launched,descendants,stage:journal.stage,oldExecutableHash,interruption:'owned NSIS tree deliberately terminated after successful launch and app exit',aclRestored:true,noAutomaticRetrySeconds:21};report.checks.push('genuine launched NSIS installation deliberately interrupted; old binary/data retained, incomplete update shown without automatic retry');
 stage('native UI and real automatic upgrade');await page.locator('aside button[aria-haspopup="dialog"]').click();await page.getByRole('button',{name:'重启并更新',exact:true}).click();await browser.close().catch(()=>{});browser=null;await delay(1800);page=await connect(testExe,false);
 await expect.poll(async()=>(await invoke(page,'update_snapshot')).build.version,{timeout:60000}).toBe('0.1.701');expect(await data(page)).toEqual(seeded);expect((await invoke(page,'update_snapshot')).state).toBe('idle');expect(componentIdentity()).toEqual(installedComponent);await page.screenshot({path:path.join(directory,'native-upgraded.png')});await close(page);report.checks.push('native UI retry completes real HTTPS/minisign/NSIS upgrade and automatic restart; target version, exact data and installed component confirmed');
 report.passed=true;
 }
 }else{
 stage('formal draft component baseline');const stableData=path.join(process.env.APPDATA,'local.sayweave.skeleton','data');if(existsSync(stableData))throw Error('Stable runner profile is not empty');
 const stableExe=path.join(process.env.LOCALAPPDATA,'Sayweave','sayweave.exe');report.installers.push(installerIdentity(stableInstaller,stableVersion,'stable'));await candidatePayload(stableInstaller);await command(stableInstaller,['/S']);const installedComponent=componentIdentity();report.componentBefore=installedComponent;expect(installedComponent.sha256).toBe(tools.manifest.artifacts.component.sha256);await installedRegistrationReady(probe);await command(probe,['installed-fixture',installedComponent.module]);
 stage('old public version and formal draft replacement');const oldAssets=path.join(directory,'old-public');mkdirSync(oldAssets);await command('gh',['release','download','v0.1.6','--repo','qinghui316/Sayweave','--pattern','*setup.exe','--dir',oldAssets]);const publicOld=path.join(oldAssets,'Sayweave_0.1.6_x64-setup.exe');report.installers.push(installerIdentity(publicOld,'0.1.6','old-public'));await command(publicOld,['/S']);
 const oldVersion=await command('powershell',['-NoProfile','-Command',`$ErrorActionPreference='Stop';$v=(Get-Item -LiteralPath ${quote(stableExe)}).VersionInfo;@{fileVersion=$v.FileVersion;productVersion=$v.ProductVersion;fileParts=@($v.FileMajorPart,$v.FileMinorPart,$v.FileBuildPart,$v.FilePrivatePart);productParts=@($v.ProductMajorPart,$v.ProductMinorPart,$v.ProductBuildPart,$v.ProductPrivatePart)}|ConvertTo-Json -Compress`],root,process.env,undefined,30000);
 report.legacyExecutableIdentity={...JSON.parse(oldVersion.stdout.trim()),sha256:createHash('sha256').update(readFileSync(stableExe)).digest('hex')};checkpoint();expect(report.legacyExecutableIdentity.fileParts).toEqual([0,1,6,0]);expect(report.legacyExecutableIdentity.productParts).toEqual([0,1,6,0]);page=await connect(stableExe,true,true);
 await expect.poll(async()=>(await invoke(page,'model_status',{retry:false})).state).toBe('ready');const saved=await invoke(page,'model_save',{request:{input:{name:'Update synthetic model',protocol:'openai-chat',baseUrl:`https://localhost:${server.address().port}/v1`,modelId:'update-fixture',usages:['text']},key:{action:'replace',value:'isolated-update-test-key'},id:null,revision:null,expectation:null}});await invoke(page,'dictionary_create',{input:{kind:'term',original:'旧版更新验收术语',category:null,replacement:null}});await close(page,false);
 const stablePath=path.join(stableData,'dictionary.sqlite'),stable=new DatabaseSync(stablePath);report.legacySeed=seedLegacyRows(stable,legacySeed,saved.configs[0].id);stable.close();
 page=await connect(stableExe,true,true);const legacyBefore=await legacyData(page,saved.configs[0]);expect(legacyBefore.dictionary.total).toBe(1);expect(legacyBefore.history.count).toBe(10);expect(legacyBefore.detail.preferences[0].rule).toBe('保留完整理由和步骤');await close(page,false);const oldDb=new DatabaseSync(stablePath,{readOnly:true});report.legacyBefore=databaseInventory(oldDb);oldDb.close();
 await candidatePayload(stableInstaller);await command(stableInstaller,['/S']);page=await connect(stableExe);const snapshot=await invoke(page,'update_snapshot');expect(snapshot.build.version).toBe(stableVersion);expect(snapshot.build.updatesEnabled).toBe(true);expect(snapshot.build.channel).toBe('stable');expect(snapshot.build.commit).toBe(commit);expect(await legacyData(page,saved.configs[0])).toEqual(legacyBefore);await page.screenshot({path:path.join(directory,'native-manual-overlay.png')});await close(page,false);const newDb=new DatabaseSync(stablePath,{readOnly:true});report.legacyAfter=databaseInventory(newDb);newDb.close();expect(report.legacyAfter).toEqual(report.legacyBefore);report.componentAfter=componentIdentity();expect(report.componentAfter).toEqual(installedComponent);await command(probe,['installed-fixture',installedComponent.module]);report.checks.push(`actual public0.1.6 database retained in place; old/native-created model and term plus bounded synthetic rows read before/after formal${stableVersion} overlay, exact table hashes/real credential value/installed TSF preserved`);report.passed=true;
 }
 }
}catch(error){report.passed=false;report.failureStage=report.stage;report.error=String(error?.message??error?.code??error);throw error;}finally{
 await cleanup('temporary unknown preference value',clearUnknownPreference);
 await cleanup('temporary machine metadata registration',clearMachineRegistration);
 await cleanup('temporary machine metadata files',clearMachineFiles);
 await cleanup('temporary production TIP registration',clearTemporaryRegistration);
 await cleanup('browser',async()=>{if(browser)await browser.close();});
 await cleanup('application',()=>{if(ownedProcess){const remaining=processes(ownedProcess.ExecutablePath).find(p=>p.ProcessId===ownedProcess.ProcessId&&p.CreationDate===ownedProcess.CreationDate);if(remaining)execFileSync('taskkill',['/PID',String(remaining.ProcessId),'/T','/F'],{stdio:'ignore'});}if(app&&app.exitCode===null)app.kill();});
 const installersStopped=await cleanup('interrupted installer tree',async()=>{
  if(deniedDirectory&&installerLaunchAttempted&&!installerTreeConfirmed)throw Error('Installer ownership snapshot unconfirmed; retain directory denial');
  if(installerTreeConfirmed&&deniedDirectory)captureInstallerTree();
  const identities=[...installerDescendants,...(installerProcess?[installerProcess]:[])];
  for(const owned of identities){const remaining=processes(owned.ExecutablePath).find(p=>p.ProcessId===owned.ProcessId&&p.CreationDate===owned.CreationDate);if(remaining)execFileSync('taskkill',['/PID',String(remaining.ProcessId),'/T','/F'],{stdio:'ignore'});}
  await expect.poll(()=>identities.some(owned=>processes(owned.ExecutablePath).some(p=>p.ProcessId===owned.ProcessId&&p.CreationDate===owned.CreationDate)),{timeout:10000}).toBe(false);
 });
 if(installersStopped)await cleanup('original installation ACL',restoreInstallAcl);
 else if(deniedDirectory)(report.cleanupErrors??=[]).push({stage:'original installation ACL',error:'Not restored while owned installer termination is unconfirmed'});
 await cleanup('temporary WebView debug policy',()=>{if(debugPolicy)debugPolicy.remove();});
 await cleanup('HTTPS server',async()=>{if(server){server.closeAllConnections();await new Promise(resolve=>server.close(resolve));}});
 await cleanup('temporary root certificate',async()=>{if(thumbprint)await command('certutil',['-delstore','Root',thumbprint],root,process.env,undefined,60000);});
 await cleanup('temporary signing key',()=>rmSync(keyPath,{force:true}));
 await cleanup('temporary TLS key',()=>{if(certificatePrivateKey)rmSync(certificatePrivateKey,{force:true});});
 // A reusable pass includes successful cleanup; never publish it before finally.
 if(report.passed&&report.kind==='mechanism')await cleanup('mechanism evidence',()=>writeMechanismEvidence(path.join(root,'.local/windows-update-evidence'),commit,tools,legacySeed,report));
 checkpoint();console.log(JSON.stringify({passed:report.passed,commit,checks:report.checks.length,receipt:path.join(directory,'acceptance.json')}));
}
