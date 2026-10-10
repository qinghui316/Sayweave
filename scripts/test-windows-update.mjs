// Real Windows/HTTPS/minisign/NSIS acceptance; only an elevated disposable Windows VM.
import {spawn,execFileSync} from 'node:child_process';
import {randomUUID,createHash} from 'node:crypto';
import {createServer} from 'node:https';
import {mkdirSync,readFileSync,writeFileSync,existsSync,symlinkSync,rmSync,renameSync} from 'node:fs';
import path from 'node:path';
import {chromium,expect} from '@playwright/test';
import {DatabaseSync} from 'node:sqlite';
import {databaseInventory,seedLegacyRows,legacySeedRows} from './windows-update-fixtures.mjs';
import {readTools,writeTools,readMechanismEvidence,writeMechanismEvidence,proofMarkers} from './windows-update-artifacts.mjs';
if(process.platform!=='win32'||process.env.GITHUB_ACTIONS!=='true'&&process.env.SAYWEAVE_DISPOSABLE_WINDOWS!=='1')throw Error('Use an elevated disposable Windows VM; never an owner machine.');
const root=process.cwd(),id=randomUUID(),directory=path.join(root,'.local','windows-update',id),source=path.join(directory,'source');mkdirSync(source,{recursive:true});
const commit=execFileSync('git',['rev-parse','HEAD'],{encoding:'utf8'}).trim(),report={schemaVersion:1,commit,environment:process.env.RUNNER_NAME??'explicit disposable Windows',id,checks:[],installers:[],commands:[],passed:false};
const argument=name=>{const n=process.argv.indexOf(name);if(n<0)return null;if(!process.argv[n+1]||process.argv[n+1].startsWith('--'))throw Error(`Missing ${name} value`);return path.resolve(process.argv[n+1]);};
const toolsOnly=process.argv.includes('--tools-only'),stableInstaller=argument('--stable-installer'),evidenceDirectory=argument('--mechanism-evidence'),toolsDirectory=argument('--tools-dir');
if(toolsOnly&&(stableInstaller||evidenceDirectory)||Boolean(stableInstaller)!==Boolean(evidenceDirectory))throw Error('Formal acceptance requires a draft installer and passed same-commit mechanism evidence');
report.kind=toolsOnly?'tools':stableInstaller?'formal-installer':'mechanism';
const log=path.join(directory,'build.log'),keyPath=path.join(directory,'fixture-signing.key'),cli=path.join(root,'node_modules','@tauri-apps','cli','tauri.js');
const delay=ms=>new Promise(resolve=>setTimeout(resolve,ms));
function checkpoint(){
 report.updatedAt=new Date().toISOString();
 for(const file of [path.join(directory,'acceptance.json'),path.join(root,'.local','windows-update-acceptance.json')]){writeFileSync(file+'.tmp',JSON.stringify(report,null,2)+'\n');renameSync(file+'.tmp',file);}
}
async function command(file,args,cwd=root,env=process.env,expectedFailure,timeoutMs=30*60_000){
 const started=Date.now(),entry={stage:report.stage,tool:path.basename(file),startedAt:new Date(started).toISOString()};report.commands.push(entry);checkpoint();
 let tail='',stdout='',stderr='';const safe=path.basename(file)==='tsf-probe.exe'||file==='cargo'&&args[0]==='build'&&args.some(arg=>arg.includes('text-service'));
 if(safe)entry.args=args;
 const diagnostics=path.join(directory,'diagnostics');if(safe)mkdirSync(diagnostics,{recursive:true});
 const child=spawn(file,args,{cwd,env,windowsHide:true,timeout:timeoutMs,stdio:['ignore','pipe','pipe']});
 for(const [label,stream] of [['stdout',child.stdout],['stderr',child.stderr]])stream.on('data',bytes=>{writeFileSync(log,bytes,{flag:'a'});tail=(tail+bytes.toString('utf8')).slice(-16384);if(label==='stdout')stdout=(stdout+bytes.toString('utf8')).slice(-131072);else stderr=(stderr+bytes.toString('utf8')).slice(-131072);});
 const heartbeat=setInterval(()=>{entry.elapsedMs=Date.now()-started;checkpoint();console.log(`Windows update acceptance: ${entry.stage} (${Math.round(entry.elapsedMs/1000)}s)`);},60000);
 let code,spawnError;child.once('error',error=>{spawnError=error;});
 try{code=await new Promise(resolve=>child.once('close',(value,signal)=>{entry.signal=signal;resolve(value);}));}finally{clearInterval(heartbeat);entry.elapsedMs=Date.now()-started;entry.exitCode=code??null;entry.timeout=child.killed;if(spawnError)entry.spawnError=spawnError.code;if(safe){entry.stdout=stdout.slice(-8192);entry.stderr=stderr.slice(-8192);for(const [label,value] of [['stdout',stdout],['stderr',stderr]])writeFileSync(path.join(diagnostics,`${report.commands.length}-${entry.tool}.${label}.log`),value);}checkpoint();}
 if(spawnError)throw spawnError;
 if(child.killed)throw Error(`Command exceeded ${timeoutMs}ms: ${path.basename(file)}`);
 if(expectedFailure){if(code!==0&&tail.includes(expectedFailure))return;throw Error('Expected build-isolation rejection was not observed');}
 if(code!==0)throw Error(`Command failed: ${file} ${args.slice(0,3).join(' ')} (${code}); ${safe?stderr.slice(-4096):'private fixture log retained'}`);
 return {stdout,stderr};
}
function stage(value){report.stage=value;checkpoint();console.log(`Windows update acceptance: ${value}`);}
let server,browser,app,thumbprint,ownedProcess,lastEndpoint,serverMode='valid',packageBytes,signature,deniedDirectory,originalAcl,installerProcess;
let installerDescendants=[];
let installerTreeConfirmed=false;
let credentialRequests=0;
let tools,legacySeed,temporaryRegistration;
async function cleanup(label,action){
 try{await action();return true;}catch(error){report.passed=false;process.exitCode=1;(report.cleanupErrors??=[]).push({stage:label,error:String(error?.message??error?.code??error).slice(0,1024)});return false;}
}
const debugPort=9291,identifier=`local.sayweave.acceptance.${id}`,product=`Sayweave 验收 ${id}`,testData=path.join(process.env.APPDATA,identifier,'data'),testExe=path.join(process.env.LOCALAPPDATA,product,'sayweave.exe');
const invoke=(page,command,args={})=>page.evaluate(({command,args})=>window.__TAURI_INTERNALS__.invoke(command,args),{command,args});
function processes(exe){
 const literal=exe.replaceAll("'","''");
 return JSON.parse(execFileSync('powershell',['-NoProfile','-Command',`$items=@(Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath -eq '${literal}' } | Select-Object ProcessId,CreationDate,ExecutablePath);ConvertTo-Json -InputObject $items -Compress`],{encoding:'utf8'}));
}
function captureInstallerTree(first=false){
 const all=JSON.parse(execFileSync('powershell',['-NoProfile','-Command','$all=@(Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId,CreationDate,ExecutablePath);ConvertTo-Json -InputObject $all -Compress'],{encoding:'utf8'}));
 const same=(a,b)=>a.ProcessId===b.ProcessId&&a.CreationDate===b.CreationDate&&a.ExecutablePath===b.ExecutablePath;
 if(first&&!all.some(p=>same(p,installerProcess)))throw Error('Installer root ended before its first verified ownership snapshot');
 const tree=[...installerDescendants,...(installerProcess?[installerProcess]:[])];
 for(let n=0;n<tree.length;n++){
  const parent=tree[n];
  if(all.some(p=>p.ProcessId===parent.ProcessId&&!same(p,parent)))throw Error('Installer parent PID reused during ownership tracking');
  for(const child of all.filter(p=>p.ParentProcessId===parent.ProcessId&&p.CreationDate>=parent.CreationDate)){
   if(!Number.isInteger(child.ProcessId)||!child.ExecutablePath)throw Error('Installer descendant identity unavailable');
   if(!tree.some(p=>same(p,child)))tree.push(child);
  }
 }
 installerDescendants=tree;if(first)installerTreeConfirmed=true;return tree;
}
async function connect(exe,launch=true,allowBroken=false){
 if(launch){if(await fetch(`http://127.0.0.1:${debugPort}/json/version`).then(()=>true).catch(()=>false))throw Error('Owned WebView port occupied');app=spawn(exe,[],{windowsHide:true,env:{...process.env,WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:`--remote-debugging-port=${debugPort} --remote-debugging-address=127.0.0.1`},stdio:'ignore'});}
 let endpoint;for(let n=0;n<600;n++){endpoint=await fetch(`http://127.0.0.1:${debugPort}/json/version`).then(r=>r.json()).catch(()=>null);if(endpoint&&(launch||endpoint.webSocketDebuggerUrl!==lastEndpoint))break;endpoint=null;await delay(100);}if(!endpoint)throw Error('New installed WebView did not start');
 const matches=processes(exe);if(matches.length!==1)throw Error('Installed process identity is not unique');if(!launch&&ownedProcess?.CreationDate===matches[0].CreationDate)throw Error('Old process cannot confirm restart');ownedProcess=matches[0];lastEndpoint=endpoint.webSocketDebuggerUrl;
 browser=await chromium.connectOverCDP(endpoint.webSocketDebuggerUrl);let page;for(let n=0;n<100;n++){page=browser.contexts().flatMap(c=>c.pages()).find(p=>p.url().includes('tauri')&&!p.url().includes('surface=overlay'));if(page)break;await delay(100);}if(!page)throw Error('Main WebView missing');await page.waitForFunction(()=>window.__TAURI_INTERNALS__?.invoke);if(!allowBroken)await expect(page.locator('aside')).toBeVisible();return page;
}
async function close(page,acceptance=true){
 if(acceptance)await invoke(page,'acceptance_update_exit').catch(()=>{});if(browser){await browser.close().catch(()=>{});browser=null;}
 if(ownedProcess){
  if(acceptance)for(let n=0;n<20;n++){if(!processes(ownedProcess.ExecutablePath).some(p=>p.CreationDate===ownedProcess.CreationDate))break;await delay(250);}
  const remaining=processes(ownedProcess.ExecutablePath).find(p=>p.CreationDate===ownedProcess.CreationDate);
  if(remaining){if(acceptance)throw Error('Native exit did not complete');execFileSync('taskkill',['/PID',String(remaining.ProcessId),'/T','/F'],{stdio:'ignore'});}
 }app=null;ownedProcess=null;await delay(600);
}
async function state(page,wanted){await expect.poll(async()=>(await invoke(page,'update_snapshot')).state,{timeout:60000}).toBe(wanted);return invoke(page,'update_snapshot');}
async function install(installer){await command(installer,['/S']);if(!existsSync(testExe))throw Error('NSIS current-user path differs');}
const data=page=>invoke(page,'acceptance_update_data');
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
async function registrationInventory(){
 const script="$ErrorActionPreference='Stop';$items=@{};foreach($scope in @('HKEY_CURRENT_USER','HKEY_LOCAL_MACHINE')){$base='Registry::'+$scope;$com=$base+'\\Software\\Classes\\CLSID\\{CB473CF0-9B4C-482C-BFC2-5527D2379ACA}';$tip=$base+'\\Software\\Microsoft\\CTF\\TIP\\{CB473CF0-9B4C-482C-BFC2-5527D2379ACA}';$item=@{com=(Test-Path -LiteralPath $com);tip=(Test-Path -LiteralPath $tip);module=$null;owner=$null};if(Test-Path -LiteralPath ($com+'\\InprocServer32')){$key=Get-Item -LiteralPath ($com+'\\InprocServer32');$item.module=$key.GetValue('');$item.owner=$key.GetValue('SayweaveOwner')};$items[$scope]=$item};ConvertTo-Json -InputObject $items -Compress";
 return JSON.parse((await command('powershell',['-NoProfile','-Command',script],root,process.env,undefined,30000)).stdout);
}
const registrationEmpty=info=>Object.values(info).every(item=>!item.com&&!item.tip);
async function awaitRegistrationState(probe,registered){
 const deadline=Date.now()+10000;let state,stable=0;
 do{
  state=JSON.parse((await command(probe,['registration-state'],root,process.env,undefined,30000)).stdout.trim());
  const matches=registered?state.profiles>0&&state.categories>0&&state.speechCategory:state.profiles===0&&state.categories===0&&!state.speechCategory;
  stable=matches?stable+1:0;if(stable===2)return state;
  await delay(250);
 }while(Date.now()<deadline);
 throw Error(`TIP registration state did not settle (${registered?'registered':'removed'}): ${JSON.stringify(state)}`);
}
function moduleKey(value){return (value.startsWith('\\\\?\\')?value.slice(4):value).toLowerCase();}
async function clearTemporaryRegistration(){
 if(!temporaryRegistration)return;
 const before=await registrationInventory(),user=before.HKEY_CURRENT_USER,machine=before.HKEY_LOCAL_MACHINE;
 if(!registrationEmpty(before)){
  if(!user.com||user.owner!=='SayweaveStageA-v1'||typeof user.module!=='string'||moduleKey(user.module)!==moduleKey(temporaryRegistration.component)||machine.com)throw Error('Temporary TIP identity is unavailable or changed; no blind removal');
  await command(temporaryRegistration.probe,['remove'],root,process.env,undefined,30000);
 }
 const after=await registrationInventory(),profiles=await awaitRegistrationState(temporaryRegistration.probe,false);
 if(!registrationEmpty(after))throw Error('Temporary TIP registration was not fully removed');
 report.toolRegistration={before,after,profiles,removed:true};temporaryRegistration=null;
}
function restoreInstallAcl(){
 if(!deniedDirectory)return;
 const script=`$acl=Get-Acl -LiteralPath ${quote(deniedDirectory)};$acl.SetSecurityDescriptorSddlForm(${quote(originalAcl)});Set-Acl -LiteralPath ${quote(deniedDirectory)} -AclObject $acl`;
 execFileSync('powershell',['-NoProfile','-Command',script],{stdio:'ignore'});
 expect(execFileSync('powershell',['-NoProfile','-Command',`(Get-Acl -LiteralPath ${quote(deniedDirectory)}).Sddl`],{encoding:'utf8'}).trim()).toBe(originalAcl);deniedDirectory=null;
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
  await page.getByRole('button',{name:'关于',exact:true}).click();const dialog=page.getByRole('dialog');await expect(dialog).toBeVisible();
  const panel=dialog.locator('section[aria-label="软件更新"]');await expect(panel).toBeVisible();
  const geometry=await dialog.evaluate(el=>{const r=el.getBoundingClientRect();return{x:r.x,y:r.y,right:r.right,bottom:r.bottom,width:innerWidth,height:innerHeight,ratio:devicePixelRatio};});
  expect(geometry.x).toBeGreaterThanOrEqual(0);expect(geometry.y).toBeGreaterThanOrEqual(0);expect(geometry.right).toBeLessThanOrEqual(geometry.width+1);expect(geometry.bottom).toBeLessThanOrEqual(geometry.height+1);
  for(const button of await panel.getByRole('button').all()){await button.scrollIntoViewIfNeeded();expect(await button.evaluate(el=>{const r=el.getBoundingClientRect();return el.contains(document.elementFromPoint(r.left+r.width/2,r.top+r.height/2));})).toBe(true);}
  const button=panel.getByRole('button').first();await button.focus();await expect(button).toBeFocused();await page.keyboard.press('Tab');expect(await dialog.evaluate(el=>el.contains(document.activeElement))).toBe(true);
  await page.screenshot({path:path.join(directory,`native-ui-${label}-${minimum?'960':'1120'}-${zoom}.png`)});await page.keyboard.press('Escape');await expect(dialog).toHaveCount(0);
  report.checks.push(`native ${label} layout ${minimum?'960x640':'1120x760'} WebView zoom${zoom}, ${motion}, focus and hit testing`);
 }
 await invoke(page,'acceptance_main_size',{minimum:false,zoom:1});await page.emulateMedia({reducedMotion:'no-preference'});
}
async function build(version,channel='test'){
 stage(`building ${channel} ${version}`);
 tools=readTools(tools.directory,commit);
 const config={version,identifier:channel==='test'?identifier:'local.sayweave.skeleton',productName:channel==='test'?product:'Sayweave',bundle:{windows:{nsis:{installMode:'currentUser'}}}},file=path.join(directory,`config-${version}-${channel}.json`);writeFileSync(file,JSON.stringify(config));
 const env={...process.env,CARGO_TARGET_DIR:path.join(root,'src-tauri','target'),SAYWEAVE_BUILD_COMMIT:commit,SAYWEAVE_BUILD_CHANNEL:channel,SAYWEAVE_BUILD_VERSION:version,SAYWEAVE_TSF_PAYLOAD:tools.component,TAURI_SIGNING_PRIVATE_KEY:keyPath,TAURI_SIGNING_PRIVATE_KEY_PASSWORD:''};
 if(channel==='test'){env.SAYWEAVE_BUILD_IDENTIFIER=identifier;env.SAYWEAVE_TEST_UPDATE_URL=`https://localhost:${server.address().port}/latest.json`;env.SAYWEAVE_TEST_UPDATE_KEY=readFileSync(`${keyPath}.pub`,'utf8').trim();}else{delete env.SAYWEAVE_BUILD_IDENTIFIER;delete env.SAYWEAVE_TEST_UPDATE_URL;delete env.SAYWEAVE_TEST_UPDATE_KEY;}
 await command(process.execPath,[path.join(source,'scripts','desktop.mjs'),'package','--config',file,...(channel==='test'?['--features','desktop-acceptance']:[])],source,env);
 readTools(tools.directory,commit);
 const installer=path.join(env.CARGO_TARGET_DIR,'release','bundle','nsis',`${config.productName}_${version}_x64-setup.exe`),copy=path.join(directory,`${channel}-${version}-setup.exe`);writeFileSync(copy,readFileSync(installer));writeFileSync(`${copy}.sig`,readFileSync(`${installer}.sig`));
 report.installers.push(installerIdentity(copy,version,channel));return copy;
}
try{
 stage('source and build isolation');
 if(execFileSync('git',['status','--porcelain','--untracked-files=no'],{encoding:'utf8'}).trim())throw Error('Acceptance requires a clean tracked source tree');
 report.compiler={rustc:execFileSync('rustc',['--version'],{encoding:'utf8'}).trim(),cargo:execFileSync('cargo',['--version'],{encoding:'utf8'}).trim()};
 await command('powershell',['-NoProfile','-Command',"$identity=[Security.Principal.WindowsIdentity]::GetCurrent();$principal=New-Object Security.Principal.WindowsPrincipal($identity);if(-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)){throw 'Disposable VM must be elevated; no interactive elevation is attempted'}"],root,process.env,undefined,30000);
 execFileSync('git',['archive','HEAD','--format=tar','-o',path.join(directory,'source.tar')]);execFileSync('tar',['-xf',path.join(directory,'source.tar'),'-C',source]);symlinkSync(path.join(root,'node_modules'),path.join(source,'node_modules'),'junction');
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
 expect(JSON.parse(initialProfiles.stdout.trim())).toEqual({profiles:0,categories:0,speechCategory:false});
 if(!registrationEmpty(initialRegistration))throw Error('Short tooling requires an empty owned TIP registration');
 temporaryRegistration={probe:tools.probe,component:tools.component};
 await command(tools.probe,['register',tools.component],root,process.env,undefined,30000);
 const prepared=await awaitRegistrationState(tools.probe,true);report.toolRegistrationPrepared=prepared;
 stage('short production DLL actual text');
 const smoke=await command(tools.probe,['installed-fixture',tools.component],root,process.env,undefined,60000);
 for(const marker of proofMarkers)expect(smoke.stdout).toContain(marker);
 stage('temporary production TIP cleanup');await clearTemporaryRegistration();
 const proof=tools.manifest?.proof??{...report.compiler,capabilities,markers:proofMarkers,registrationClean:true};
 tools=writeTools(path.join(root,'.local/windows-update-tools'),commit,readFileSync(tools.component),readFileSync(tools.probe),proof);report.tools=tools.manifest;
 report.checks.push('separate production DLL/acceptance probe; capabilities, actual protocol/control text, absent fault export and final COM unload passed');
 if(toolsOnly){report.passed=true;}
 else{
 if(!stableInstaller){stage('stable feature isolation');await command('cargo',['check','--manifest-path','src-tauri/Cargo.toml','--features','desktop-acceptance'],root,{...process.env,SAYWEAVE_BUILD_CHANNEL:'stable'},'acceptance feature in stable build');report.checks.push('actual stable build rejects the desktop-acceptance feature');}
 stage('HTTPS certificate fixture');await command('cargo',['test','--manifest-path','src-tauri/Cargo.toml','model_desktop_fixture','--','--ignored'],root,{...process.env,SAYWEAVE_DATA_ACCEPTANCE_ID:id});
 const certRoot=path.join(root,'.local','data-acceptance',id,'data'),certificateFile=path.join(certRoot,'fixture-ca.der');
 thumbprint=execFileSync('powershell',['-NoProfile','-Command',`(New-Object System.Security.Cryptography.X509Certificates.X509Certificate2(${quote(certificateFile)})).Thumbprint`],{encoding:'utf8',timeout:30000}).trim();
 stage('isolated HTTPS trust import');
 await command('powershell',['-NoProfile','-Command',`$ErrorActionPreference='Stop';$cert=New-Object System.Security.Cryptography.X509Certificates.X509Certificate2(${quote(certificateFile)});$store=New-Object System.Security.Cryptography.X509Certificates.X509Store('Root','LocalMachine');try{$store.Open('ReadWrite');$store.Add($cert);if($store.Certificates.Find('FindByThumbprint',${quote(thumbprint)},$false).Count -ne 1){throw 'Isolated root import not confirmed'}}finally{$store.Close();$cert.Dispose()}`],root,process.env,undefined,60000);
 report.trust={store:'LocalMachine/Root',thumbprint};report.checks.push('disposable elevated VM root imported without CurrentUser protected-root interaction; exact thumbprint confirmed');
 if(!stableInstaller){stage('temporary signature key');await command(process.execPath,[cli,'signer','generate','--ci','--write-keys',keyPath],root,process.env,undefined,60000);}
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
 });await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
 let page;const sid=execFileSync('powershell',['-NoProfile','-Command','[Security.Principal.WindowsIdentity]::GetCurrent().User.Value'],{encoding:'utf8'}).trim();
 const probe=tools.probe;
 if(!stableInstaller){
 const old=await build('0.1.700'),next=await build('0.1.701');packageBytes=readFileSync(next);signature=readFileSync(`${next}.sig`,'utf8').trim();
 stage('real NSIS manual replacement');await install(old);
 await command(testExe,['--sayweave-text-service=install',`--initiator=${sid}`]);
 const installedComponent=componentIdentity();report.componentBefore=installedComponent;expect(installedComponent.sha256).toBe(tools.manifest.artifacts.component.sha256);
 await command(probe,['installed-fixture',installedComponent.module]);
 page=await connect(testExe);const seeded=await invoke(page,'acceptance_update_seed');expect(seeded.credentialMatches).toBe(true);expect(seeded.history.count).toBe(10);expect(seeded.profile.preferenceCount).toBe(1);expect(seeded.schema).toBe(9);expect(seeded.component.installed).toBe(true);expect(seeded.component.updateAvailable).toBe(false);expect(seeded.component.error).toBeNull();
 await close(page);const seedDb=new DatabaseSync(path.join(testData,'dictionary.sqlite'),{readOnly:true});legacySeed=legacySeedRows(seedDb);seedDb.close();await install(next);page=await connect(testExe);expect((await invoke(page,'update_snapshot')).build.version).toBe('0.1.701');expect(await data(page)).toEqual(seeded);await close(page);report.checks.push('real NSIS overlay preserves credential value, schema9, history, dictionary, profile and component state');
 stage('real HTTPS failures and cancellation');await install(old);page=await connect(testExe);
 for(const mode of ['interrupted','bad-signature','wrong-version']){serverMode=mode;await invoke(page,'update_check',{manual:true});await state(page,'failed');expect(await data(page)).toEqual(seeded);report.checks.push(`real HTTPS ${mode} rejected; native app/data remain usable`);}
 await nativeLayout(page,'failure');
 serverMode='slow';await invoke(page,'update_check',{manual:true});await state(page,'downloading');await invoke(page,'update_cancel');await delay(500);expect((await invoke(page,'update_snapshot')).state).toBe('idle');report.checks.push('real streaming cancellation rejects late completion');serverMode='valid';let candidate=await ready(page);
 await nativeLayout(page,'ready');stage('preparation, shutdown and launch recovery');await invoke(page,'acceptance_update_hold',{kind:'mutation'});await invoke(page,'update_install',{taskId:candidate.taskId,revision:candidate.revision});candidate=await state(page,'readyToInstall');expect(candidate.failureStage).toBe('prepareTimeout');await delay(2500);expect(await data(page)).toEqual(seeded);report.checks.push('admitted mutation exceeds preparation deadline; fence releases and data remains usable');
 await invoke(page,'acceptance_update_hold',{kind:'stop'});await invoke(page,'update_install',{taskId:candidate.taskId,revision:candidate.revision});await state(page,'recovery');report.checks.push('held service stop lock exceeds five seconds; no installation; recovery displayed');
 await invoke(page,'update_restart');await browser.close().catch(()=>{});browser=null;await delay(1800);page=await connect(testExe,false);candidate=await state(page,'readyToInstall');expect((await invoke(page,'update_snapshot')).build.version).toBe('0.1.700');expect(await data(page)).toEqual(seeded);report.checks.push('restart of old binary is not update success');
 const cache=path.join(testData,'update-cache','candidate.exe'),user=execFileSync('whoami',[],{encoding:'utf8'}).trim();await command('icacls',[cache,'/deny',`${user}:(X)`]);await invoke(page,'update_install',{taskId:candidate.taskId,revision:candidate.revision});await state(page,'recovery');await command('icacls',[cache,'/remove:d',user]);report.checks.push('real Windows execute denial produces launch recovery before process exit');
 stage('real launched installer interruption');await invoke(page,'update_restart');await browser.close().catch(()=>{});browser=null;await delay(1800);page=await connect(testExe,false);candidate=await state(page,'readyToInstall');
 const oldExecutableHash=createHash('sha256').update(readFileSync(testExe)).digest('hex');
 deniedDirectory=path.dirname(testExe);originalAcl=execFileSync('powershell',['-NoProfile','-Command',`(Get-Acl -LiteralPath ${quote(deniedDirectory)}).Sddl`],{encoding:'utf8'}).trim();
 await command('icacls',[deniedDirectory,'/deny',`${user}:(OI)(CI)(W,D)`]);
 let denied=false;try{writeFileSync(path.join(deniedDirectory,'must-not-create.tmp'),'forbidden');}catch{denied=true;}expect(denied).toBe(true);
 const originalProcess=ownedProcess,installing=invoke(page,'update_install',{taskId:candidate.taskId,revision:candidate.revision}).catch(()=>{});
 await expect.poll(()=>{installerProcess=processes(cache)[0];if(installerProcess)captureInstallerTree(true);return Boolean(installerProcess);},{timeout:15000,intervals:[100]}).toBe(true);
 await expect.poll(()=>{captureInstallerTree();return processes(testExe).some(p=>p.CreationDate===originalProcess.CreationDate);},{timeout:15000,intervals:[100]}).toBe(false);
 const launched=installerProcess,descendants=captureInstallerTree();
 for(const owned of descendants){const remaining=processes(owned.ExecutablePath).find(p=>p.ProcessId===owned.ProcessId&&p.CreationDate===owned.CreationDate);if(remaining)execFileSync('taskkill',['/PID',String(remaining.ProcessId),'/T','/F'],{stdio:'ignore'});}
 await expect.poll(()=>descendants.some(p=>processes(p.ExecutablePath).some(current=>current.ProcessId===p.ProcessId&&current.CreationDate===p.CreationDate)),{timeout:10000}).toBe(false);installerProcess=null;
 await installing;await browser.close().catch(()=>{});browser=null;ownedProcess=null;
 expect(createHash('sha256').update(readFileSync(testExe)).digest('hex')).toBe(oldExecutableHash);
 const journal=JSON.parse(readFileSync(path.join(testData,'update-cache','transaction.json'),'utf8'));expect(['launching','launched']).toContain(journal.stage);
 restoreInstallAcl();page=await connect(testExe);candidate=await state(page,'readyToInstall');expect((await invoke(page,'update_snapshot')).build.version).toBe('0.1.700');expect(candidate.error).toContain('上次更新未完成');expect(await data(page)).toEqual(seeded);
 for(let n=0;n<21;n++){expect(processes(cache)).toHaveLength(0);await delay(1000);}expect((await invoke(page,'update_snapshot')).state).toBe('readyToInstall');
 report.interruptedInstaller={...installerIdentity(cache,'0.1.701','test'),process:launched,descendants,stage:journal.stage,oldExecutableHash,interruption:'owned NSIS tree deliberately terminated after successful launch and app exit',aclRestored:true,noAutomaticRetrySeconds:21};report.checks.push('genuine launched NSIS installation deliberately interrupted; old binary/data retained, incomplete update shown without automatic retry');
 stage('native UI and real automatic upgrade');await page.getByRole('button',{name:'关于',exact:true}).click();await page.getByRole('button',{name:'重启并更新',exact:true}).click();await browser.close().catch(()=>{});browser=null;await delay(1800);page=await connect(testExe,false);
 await expect.poll(async()=>(await invoke(page,'update_snapshot')).build.version,{timeout:60000}).toBe('0.1.701');expect(await data(page)).toEqual(seeded);expect((await invoke(page,'update_snapshot')).state).toBe('idle');expect(componentIdentity()).toEqual(installedComponent);await page.screenshot({path:path.join(directory,'native-upgraded.png')});await close(page);report.checks.push('native UI retry completes real HTTPS/minisign/NSIS upgrade and automatic restart; target version, exact data and installed component confirmed');
 report.passed=true;
 }else{
 stage('formal draft component baseline');const stableData=path.join(process.env.APPDATA,'local.sayweave.skeleton','data');if(existsSync(stableData))throw Error('Stable runner profile is not empty');
 const stableExe=path.join(process.env.LOCALAPPDATA,'Sayweave','sayweave.exe');report.installers.push(installerIdentity(stableInstaller,'0.1.7','stable'));await command(stableInstaller,['/S']);await command(stableExe,['--sayweave-text-service=install',`--initiator=${sid}`]);const installedComponent=componentIdentity();report.componentBefore=installedComponent;expect(installedComponent.sha256).toBe(tools.manifest.artifacts.component.sha256);await command(probe,['installed-fixture',installedComponent.module]);
 stage('old public version and formal draft replacement');const oldAssets=path.join(directory,'old-public');mkdirSync(oldAssets);await command('gh',['release','download','v0.1.6','--repo','qinghui316/Sayweave','--pattern','*setup.exe','--dir',oldAssets]);const publicOld=path.join(oldAssets,'Sayweave_0.1.6_x64-setup.exe');report.installers.push(installerIdentity(publicOld,'0.1.6','old-public'));await command(publicOld,['/S']);page=await connect(stableExe,true,true);
 await expect.poll(async()=>(await invoke(page,'model_status',{retry:false})).state).toBe('ready');const saved=await invoke(page,'model_save',{request:{input:{name:'Update synthetic model',protocol:'openai-chat',baseUrl:`https://localhost:${server.address().port}/v1`,modelId:'update-fixture',usages:['text']},key:{action:'replace',value:'isolated-update-test-key'},id:null,revision:null,expectation:null}});await invoke(page,'dictionary_create',{input:{kind:'term',original:'旧版更新验收术语',category:null,replacement:null}});await close(page,false);
 const stablePath=path.join(stableData,'dictionary.sqlite'),stable=new DatabaseSync(stablePath);report.legacySeed=seedLegacyRows(stable,legacySeed,saved.configs[0].id);stable.close();
 page=await connect(stableExe,true,true);const legacyBefore=await legacyData(page,saved.configs[0]);expect(legacyBefore.dictionary.total).toBe(1);expect(legacyBefore.history.count).toBe(10);expect(legacyBefore.detail.preferences[0].rule).toBe('保留完整理由和步骤');await close(page,false);const oldDb=new DatabaseSync(stablePath,{readOnly:true});report.legacyBefore=databaseInventory(oldDb);oldDb.close();
 await command(stableInstaller,['/S']);page=await connect(stableExe);const snapshot=await invoke(page,'update_snapshot');expect(snapshot.build.version).toBe('0.1.7');expect(snapshot.build.updatesEnabled).toBe(true);expect(snapshot.build.channel).toBe('stable');expect(snapshot.build.commit).toBe(commit);expect(await legacyData(page,saved.configs[0])).toEqual(legacyBefore);await page.screenshot({path:path.join(directory,'native-manual-overlay.png')});await close(page,false);const newDb=new DatabaseSync(stablePath,{readOnly:true});report.legacyAfter=databaseInventory(newDb);newDb.close();expect(report.legacyAfter).toEqual(report.legacyBefore);report.componentAfter=componentIdentity();expect(report.componentAfter).toEqual(installedComponent);await command(probe,['installed-fixture',installedComponent.module]);report.checks.push('actual public0.1.6 database retained in place; old/native-created model and term plus bounded synthetic rows read before/after formal0.1.7 overlay, exact table hashes/real credential value/installed TSF preserved');report.passed=true;
 }
 }
}catch(error){report.passed=false;report.failureStage=report.stage;report.error=String(error?.message??error?.code??error);throw error;}finally{
 await cleanup('temporary production TIP registration',clearTemporaryRegistration);
 await cleanup('browser',async()=>{if(browser)await browser.close();});
 await cleanup('application',()=>{if(ownedProcess){const remaining=processes(ownedProcess.ExecutablePath).find(p=>p.ProcessId===ownedProcess.ProcessId&&p.CreationDate===ownedProcess.CreationDate);if(remaining)execFileSync('taskkill',['/PID',String(remaining.ProcessId),'/T','/F'],{stdio:'ignore'});}if(app&&app.exitCode===null)app.kill();});
 const installersStopped=await cleanup('interrupted installer tree',async()=>{
  if(deniedDirectory&&!installerTreeConfirmed)throw Error('Installer ownership snapshot unconfirmed; retain directory denial');
  if(installerTreeConfirmed&&deniedDirectory)captureInstallerTree();
  const identities=[...installerDescendants,...(installerProcess?[installerProcess]:[])];
  for(const owned of identities){const remaining=processes(owned.ExecutablePath).find(p=>p.ProcessId===owned.ProcessId&&p.CreationDate===owned.CreationDate);if(remaining)execFileSync('taskkill',['/PID',String(remaining.ProcessId),'/T','/F'],{stdio:'ignore'});}
  await expect.poll(()=>identities.some(owned=>processes(owned.ExecutablePath).some(p=>p.ProcessId===owned.ProcessId&&p.CreationDate===owned.CreationDate)),{timeout:10000}).toBe(false);
 });
 if(installersStopped)await cleanup('original installation ACL',restoreInstallAcl);
 else if(deniedDirectory)(report.cleanupErrors??=[]).push({stage:'original installation ACL',error:'Not restored while owned installer termination is unconfirmed'});
 await cleanup('HTTPS server',async()=>{if(server){server.closeAllConnections();await new Promise(resolve=>server.close(resolve));}});
 await cleanup('temporary root certificate',async()=>{if(thumbprint)await command('certutil',['-delstore','Root',thumbprint],root,process.env,undefined,60000);});
 await cleanup('temporary signing key',()=>rmSync(keyPath,{force:true}));
 // A reusable pass includes successful cleanup; never publish it before finally.
 if(report.passed&&report.kind==='mechanism')await cleanup('mechanism evidence',()=>writeMechanismEvidence(path.join(root,'.local/windows-update-evidence'),commit,tools,legacySeed,report));
 checkpoint();console.log(JSON.stringify({passed:report.passed,commit,checks:report.checks.length,receipt:path.join(directory,'acceptance.json')}));
}
