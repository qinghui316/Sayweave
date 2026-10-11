// Focused missing cases on an elevated, disposable hosted Windows runner.
// Uses the original signed installer/helper and an unchanged older production DLL.
import {spawn} from 'node:child_process';
import {createHash} from 'node:crypto';
import {copyFileSync,existsSync,mkdirSync,readFileSync,writeFileSync} from 'node:fs';
import path from 'node:path';
import {expect} from '@playwright/test';
import {readTools,proofMarkers} from './windows-update-artifacts.mjs';
import {aclPrelude} from './windows-update-startup.mjs';
const hash=bytes=>createHash('sha256').update(bytes).digest('hex');
export const olderProduct='abc3eed9375b3c170e7b144158ca24bb632fd41e';
const classId='{CB473CF0-9B4C-482C-BFC2-5527D2379ACA}',profileId='{6CD26440-E770-471E-8212-B5A565E816F5}';
const quote=value=>"'"+value.replaceAll("'","''")+"'";
function disposable(){if(process.platform!=='win32'||process.env.GITHUB_ACTIONS!=='true'||process.env.RUNNER_ENVIRONMENT!=='github-hosted')throw Error('Boundary checks require disposable hosted Windows');}
export function boundaryCheckpoint(kind,reading){
 if(kind==='prepared')return reading.user.phase==='prepared'&&reading.machine.phase==='prepared';
 if(kind==='program-recovery')return reading.user.phase==='recovery'&&reading.user.lastFailure?.phase==='programInstalled'&&reading.user.lastFailure?.code==='acceptanceInstallerProgramFailure'&&reading.machine.phase==='prepared';
 if(kind==='machine-recovery')return reading.user.phase==='recovery'&&reading.user.lastFailure?.phase==='machineCommitted'&&reading.user.lastFailure?.code==='componentUserInitialize'&&reading.user.userInitializationStarted===true&&reading.machine.phase==='machineCommitted';
 return false;
}
export function preciseProcessRecord(text,role){
 if(!['server','installer'].includes(role)||text.length>128*1024)throw Error('Invalid fixed process record');
 const value=JSON.parse(text.replace(/("created"\s*:\s*)(\d+)/g,'$1"$2"'))[role];
 if(!value||!Number.isSafeInteger(value.pid)||value.pid<1||typeof value.created!=='string'||!/^\d{1,20}$/.test(value.created)||!/^[a-f0-9]{64}$/.test(value.imageHash))throw Error('Precise native process identity missing');
 return {pid:value.pid,created:value.created,imageHash:value.imageHash};
}
const processBirth=String.raw`Add-Type -TypeDefinition 'using System;using System.Runtime.InteropServices;public static class BoundaryBirth{[StructLayout(LayoutKind.Sequential)]public struct FT{public uint lo,hi;}[DllImport("kernel32")]static extern IntPtr OpenProcess(uint access,bool inherit,uint pid);[DllImport("kernel32")]static extern bool GetProcessTimes(IntPtr process,out FT created,out FT exited,out FT kernel,out FT user);[DllImport("kernel32")]static extern bool TerminateProcess(IntPtr h,uint code);[DllImport("kernel32")]static extern bool CloseHandle(IntPtr h);public static void Stop(uint pid,string expected){var h=OpenProcess(0x1001,false,pid);if(h==IntPtr.Zero)throw new Exception("Process query failed");try{FT a,b,c,d;if(!GetProcessTimes(h,out a,out b,out c,out d))throw new Exception("Process birth query failed");if((((ulong)a.hi<<32)|a.lo).ToString()!=expected)throw new Exception("Original native process identity changed");if(!TerminateProcess(h,1))throw new Exception("Owned termination failed");}finally{CloseHandle(h);}}}' ;`;
async function stopProcess(ctx,expected){
 const result=await ctx.command('powershell',['-NoProfile','-Command',`$ErrorActionPreference='Stop';$p=Get-CimInstance Win32_Process -Filter 'ProcessId=${expected.pid}';if(-not $p){'absent';exit};${processBirth}$hash=(Get-FileHash -LiteralPath $p.ExecutablePath -Algorithm SHA256).Hash.ToLowerInvariant();if($hash -cne ${quote(expected.imageHash)}){throw 'Original native process image changed; termination refused'};[BoundaryBirth]::Stop(${expected.pid},${quote(expected.created)});'terminated'`]);
 return {...expected,alreadyExited:result.trim()==='absent'};
}
export async function prepareOlderComponent(ctx){
 disposable();const old=readTools('.local/prior-old-component-tools',olderProduct);
 const expected=old.manifest.artifacts.component.sha256;
 if(expected===ctx.packages.manifest.componentSha256)throw Error('Older component must have a distinct real payload');
 const module=path.join(process.env.ProgramFiles,'Sayweave','TextService',expected,'sayweave_text_service.dll');
 await ctx.command('powershell',['-NoProfile','-Command',`$ErrorActionPreference='Stop';foreach($base in @('HKEY_LOCAL_MACHINE','HKEY_CURRENT_USER')){foreach($suffix in @('Software\\Classes\\CLSID\\${classId}','Software\\Microsoft\\CTF\\TIP\\${classId}')){if(Test-Path -LiteralPath ('Registry::'+$base+'\\'+$suffix)){throw 'Older component fixture requires empty registration'}}};$path=${quote(module)};$parent=Split-Path $path -Parent;foreach($item in @($parent,(Split-Path $parent -Parent),(Split-Path (Split-Path $parent -Parent) -Parent))){if(Test-Path -LiteralPath $item){if((Get-Item -LiteralPath $item).Attributes -band [IO.FileAttributes]::ReparsePoint){throw 'Older fixture link rejected'}}};'empty'`]);
 if(existsSync(module))throw Error('Older component fixture path already exists');
 mkdirSync(path.dirname(module),{recursive:true});copyFileSync(old.component,module);
 expect(hash(readFileSync(module))).toBe(expected);
 await ctx.command(old.probe,['register-machine',module]);
 const before=await ctx.componentInventory();
 expect(before.HKEY_LOCAL_MACHINE.module.replace(/^\\\\\?\\/,'').toLowerCase()).toBe(module.toLowerCase());
 // Confirm actual production TSF behavior before holding the old COM instance.
 const oldText=await ctx.command(old.probe,['installed-fixture',module]);
 for(const marker of proofMarkers)expect(oldText).toContain(marker);
 const file=path.join(ctx.directory,'older-component-host.ps1');
 writeFileSync(file,String.raw`param([string]$Module)
$ErrorActionPreference='Stop';[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
Add-Type -TypeDefinition @'
using System;using System.Runtime.InteropServices;
public static class OldComponentLease {
 [DllImport("kernel32",CharSet=CharSet.Unicode)]public static extern IntPtr LoadLibrary(string name);
 [DllImport("kernel32")]public static extern bool FreeLibrary(IntPtr module);
 [DllImport("kernel32",CharSet=CharSet.Ansi)]public static extern IntPtr GetProcAddress(IntPtr module,string name);
 [DllImport("ole32")]public static extern int CoInitializeEx(IntPtr reserved,uint mode);
 [DllImport("ole32")]public static extern void CoUninitialize();
 [DllImport("ole32")]public static extern int CoCreateInstance(ref Guid cls,IntPtr outer,uint context,ref Guid iid,out IntPtr value);
 [UnmanagedFunctionPointer(CallingConvention.StdCall)]public delegate int CanUnload();
}
'@
$initialized=[OldComponentLease]::CoInitializeEx([IntPtr]::Zero,2);if($initialized -lt 0){throw 'COM apartment failed'}
$library=[OldComponentLease]::LoadLibrary($Module);if($library -eq [IntPtr]::Zero){throw 'Older DLL did not load'}
$instance=[IntPtr]::Zero
try{
 $cls=[Guid]'`+classId+String.raw`';$iid=[Guid]'00000000-0000-0000-C000-000000000046'
 $result=[OldComponentLease]::CoCreateInstance([ref]$cls,[IntPtr]::Zero,1,[ref]$iid,[ref]$instance);if($result -lt 0 -or $instance -eq [IntPtr]::Zero){throw 'Old production COM instance absent'}
 $canUnload=[Runtime.InteropServices.Marshal]::GetDelegateForFunctionPointer([OldComponentLease]::GetProcAddress($library,'DllCanUnloadNow'),[OldComponentLease+CanUnload])
 do{
  $reading=@{pid=$PID;module=$Module;sha256=(Get-FileHash -LiteralPath $Module -Algorithm SHA256).Hash.ToLowerInvariant();canUnload=$canUnload.Invoke();comLease=($instance -ne [IntPtr]::Zero)}
  [Console]::WriteLine('LEASE '+($reading|ConvertTo-Json -Compress));[Console]::Out.Flush()
  $request=[Console]::ReadLine()
 }while($request -eq 'observe')
}finally{if($instance -ne [IntPtr]::Zero){[void][Runtime.InteropServices.Marshal]::Release($instance)};[void][OldComponentLease]::FreeLibrary($library);[OldComponentLease]::CoUninitialize()}
[Console]::WriteLine('LEASE_RELEASED')
`,'utf8');
 ctx.report.olderComponent={productCommit:olderProduct,sha256:expected,module,preparation:'Unchanged historical production DLL/tooling; old fixture preparation only',registrationBefore:before,passed:false};ctx.persist();
 const host=spawn('powershell',['-NoProfile','-ExecutionPolicy','Bypass','-File',file,'-Module',module],{windowsHide:true,stdio:['pipe','pipe','pipe']});
 const readings=[];let output='',error='';host.stdout.on('data',bytes=>{output+=bytes;const lines=output.split('\n');output=lines.pop();for(const line of lines.filter(line=>line.startsWith('LEASE ')))readings.push(JSON.parse(line.slice(6)));});host.stderr.on('data',bytes=>{error=(error+bytes).slice(-4096);ctx.report.olderComponent.hostError=error;ctx.persist();});
 ctx.oldLease={host,module,expected,readings,get error(){return error;}};
 await expect.poll(()=>readings.length,{timeout:15000}).toBeGreaterThan(0);
 expect(readings[0]).toMatchObject({sha256:expected,comLease:true,canUnload:1});
 ctx.report.olderComponent.leaseBefore=readings[0];ctx.persist();
}
export async function releaseOlderComponent(ctx){
 if(!ctx.oldLease)return;const {host}=ctx.oldLease;
 if(host.exitCode===null){host.stdin.end('release\n');await new Promise((resolve,reject)=>{const timer=setTimeout(()=>reject(Error('Old COM lease release exceeded deadline')),10000);host.once('close',code=>{clearTimeout(timer);if(code===0)resolve();else reject(Error('Old COM lease failed to release'));});});}
 ctx.report.olderComponent.leaseReleased=true;ctx.persist();
}
export async function observeMappedTransition(ctx){
 const lease=ctx.oldLease;lease.host.stdin.write('observe\n');await expect.poll(()=>lease.readings.length,{timeout:5000}).toBeGreaterThan(1);
 const reading=lease.readings.at(-1),after=await ctx.componentInventory();
 expect(lease.host.exitCode).toBe(null);expect(reading).toMatchObject({sha256:lease.expected,comLease:true,canUnload:1});
 const module=after.HKEY_LOCAL_MACHINE.module;
 expect(hash(readFileSync(module))).toBe(ctx.packages.manifest.componentSha256);expect(module.toLowerCase()).not.toBe(lease.module.toLowerCase());
 expect(hash(readFileSync(lease.module))).toBe(lease.expected);
 const current=readTools('.local/prior-update-tools',ctx.candidate),text=await ctx.command(current.probe,['installed-fixture',module]);for(const marker of proofMarkers)expect(text).toContain(marker);
 Object.assign(ctx.report.olderComponent,{registrationAfter:after,leaseAfter:reading,newModuleSha256:ctx.packages.manifest.componentSha256,newActualTextPassed:true,oldFilePreservedWhileHeld:true,passed:true});ctx.persist();
 await releaseOlderComponent(ctx);
}
const tipRoot='Registry::HKEY_CURRENT_USER\\Software\\Microsoft\\CTF\\TIP\\'+classId;
export async function installerBoundaryCases(ctx){
 const results=ctx.report.installerBoundaries=[];
 let page=await ctx.connect();const seed=await ctx.invoke(page,'acceptance_update_seed');expect(seed.credentialMatches).toBe(true);
 await ctx.invoke(page,'text_service_action',{action:'disable'});const before=await ctx.invoke(page,'acceptance_update_data');await ctx.closeCurrent(page);
 const helper=path.join(ctx.extracted,'$PLUGINSDIR','sayweave-maintenance.exe'),main=path.join(ctx.extracted,'$PLUGINSDIR','sayweave-main.exe');
 for(const kind of ['prepared','program-recovery','machine-recovery']){
  ctx.stage('focused abandoned '+kind);
  const test={kind,passed:false};results.push(test);ctx.persist();
  const leaf=tipRoot+'\\LanguageProfile\\0x'+ctx.deploymentReading().user.language.toString(16).toUpperCase().padStart(8,'0')+'\\'+profileId;
  if(kind==='machine-recovery'){
   page=await ctx.connect();await ctx.invoke(page,'text_service_action',{action:'enable'});await ctx.closeCurrent(page);
   await ctx.command('powershell',['-NoProfile','-Command',`$ErrorActionPreference='Stop';$key=Get-Item -LiteralPath ${quote(leaf)};if($key.GetValue('Enable') -ne 1){throw 'Fixture Enable is not owned true'};Remove-ItemProperty -LiteralPath ${quote(leaf)} -Name Enable`]);
  }
  const driverFile=path.join(ctx.directory,'driver-'+kind+'.ps1');
  writeFileSync(driverFile,`$ErrorActionPreference='Stop';function Run-Helper([string]$action){& ${quote(helper)} ("--sayweave-installer="+$action) ${quote('--install-dir='+path.dirname(ctx.exe))} ("--installer-pid="+$PID) ${quote('--product-key=Software\\sayweave\\'+ctx.packages.manifest.product)} ${quote('--main-source='+main)};return $LASTEXITCODE};$result=Run-Helper 'begin';if($result -ne 0){exit $result};[Console]::WriteLine('PREPARED');[Console]::Out.Flush();$null=[Console]::ReadLine();${kind==='program-recovery'?"$env:SAYWEAVE_INSTALL_ACCEPTANCE_FAULT='after-program';":''}$result=Run-Helper 'commit';[Console]::WriteLine('COMMIT_EXIT '+$result);[Console]::Out.Flush();$null=[Console]::ReadLine();exit $result`,'utf8');
  ctx.ownedBoundaryServer=null;ctx.ownedBoundaryController=null;
  const driver=spawn('powershell',['-NoProfile','-ExecutionPolicy','Bypass','-File',driverFile],{windowsHide:true,stdio:['pipe','pipe','pipe']});let text='',errors='';driver.stdout.on('data',bytes=>{text+=bytes;});driver.stderr.on('data',bytes=>{errors=(errors+bytes).slice(-2048);});
  ctx.boundaryDriver=driver;test.driverPid=driver.pid;
  await expect.poll(()=>text.includes('PREPARED')?'prepared':driver.exitCode!==null?'exited:'+driver.exitCode+':'+errors:'waiting',{timeout:15000}).toBe('prepared');
  test.prepared=ctx.deploymentReading();ctx.persist();expect(boundaryCheckpoint('prepared',test.prepared)).toBe(true);
  const preparedRaw=readFileSync(path.join(process.env.ProgramFiles,'Sayweave','TextService','installer.json'),'utf8');
  ctx.ownedBoundaryServer=preciseProcessRecord(preparedRaw,'server');ctx.ownedBoundaryController=preciseProcessRecord(preparedRaw,'installer');
  expect(ctx.ownedBoundaryController.pid).toBe(driver.pid);expect(ctx.ownedBoundaryServer.imageHash).toBe(ctx.report.helperIdentity.helperSha256);
  try{
   if(kind==='machine-recovery'){
    const snapshot=await ctx.command('powershell',['-NoProfile','-Command',aclPrelude+`$key=${quote(tipRoot)};$acl=Get-Acl -LiteralPath $key;@{binary=[Convert]::ToBase64String($acl.GetSecurityDescriptorBinaryForm())}|ConvertTo-Json -Compress`]);test.registryAcl=JSON.parse(snapshot);ctx.registryAcl={key:tipRoot,...test.registryAcl};ctx.persist();
    await ctx.command('powershell',['-NoProfile','-Command',aclPrelude+`$key=${quote(tipRoot)};$acl=Get-Acl -LiteralPath $key;$sid=[Security.Principal.WindowsIdentity]::GetCurrent().User;$rule=[Security.AccessControl.RegistryAccessRule]::new($sid,[Security.AccessControl.RegistryRights]6,[Security.AccessControl.InheritanceFlags]::ContainerInherit,[Security.AccessControl.PropagationFlags]::None,[Security.AccessControl.AccessControlType]::Deny);$acl.AddAccessRule($rule);Set-Acl -LiteralPath $key -AclObject $acl;`]);
   }
   if(kind!=='prepared'){driver.stdin.write('commit\n');await expect.poll(()=>text.includes('COMMIT_EXIT 1')?'failed':driver.exitCode!==null?'exited':'waiting',{timeout:20000}).toBe('failed');}
   test.interrupted=ctx.deploymentReading();ctx.persist();expect(boundaryCheckpoint(kind,test.interrupted)).toBe(true);
   // Terminate the verified elevated server first, then the owned controller.
   const raw=readFileSync(path.join(process.env.ProgramFiles,'Sayweave','TextService','installer.json'),'utf8');
   const server=preciseProcessRecord(raw,'server'),controller=preciseProcessRecord(raw,'installer');
   expect(server.pid).toBe(test.interrupted.machine.server.pid);expect(server.imageHash).toBe(ctx.report.helperIdentity.helperSha256);expect(controller.pid).toBe(driver.pid);
   expect(server).toEqual(ctx.ownedBoundaryServer);expect(controller).toEqual(ctx.ownedBoundaryController);
   test.originalNativeProcesses={server,controller};ctx.persist();
   test.killedServer=await stopProcess(ctx,server);
   test.killedController=await stopProcess(ctx,controller);ctx.persist();
  }finally{if(ctx.registryAcl)await restoreRegistryAcl(ctx);}
  const abandoned=ctx.deploymentReading();expect(abandoned.user.phase).not.toBe('committed');test.abandoned=abandoned;ctx.persist();
  // Actual next-entry NSIS performs recovery; no receipt/database editing here.
  await ctx.attemptInstallation(ctx.packages.old);test.reentered=ctx.deploymentReading();ctx.persist();
  expect(test.reentered.user.phase).toBe('committed');expect(test.reentered.user.transaction).not.toBe(test.interrupted.user.transaction);expect(test.reentered.machine.phase).toBe('committed');
  page=await ctx.connect();const after=await ctx.invoke(page,'acceptance_update_data');
  if(kind==='machine-recovery')expect(after).toEqual({...before,component:{...before.component,enabled:true}});else expect(after).toEqual(before);
  await ctx.closeCurrent(page);test.dataAndCredentialPreserved=true;test.passed=true;ctx.persist();
 }
 ctx.stage('real native admission error is not a quiet empty result');page=await ctx.connect();await page.getByRole('link',{name:'首页',exact:true}).click();
 const admission=await ctx.invoke(page,'speech_start').then(()=>({unexpectedAdmission:true})).catch(error=>({code:error.code??String(error)}));ctx.report.emptyResultGuard={admission,quietSuccessDeferred:'Hosted runner lacks an interactive microphone environment'};ctx.persist();
 expect(admission.unexpectedAdmission).toBeUndefined();expect(['deviceMissing','deviceAccess','notConfigured']).toContain(admission.code);
 await page.getByRole('button',{name:'开始录音',exact:true}).click();
 const messages={deviceMissing:'所选麦克风不可用，请在设置中重新选择。',deviceAccess:'无法访问麦克风，请检查 Windows 麦克风权限和设备。',notConfigured:'请先在模型设置中选择语音模型。'};
 await expect(page.getByText(messages[admission.code],{exact:true})).toBeVisible();
 expect(await page.getByText('未识别到文字。',{exact:true}).count()).toBe(0);
 await page.screenshot({path:path.join(ctx.directory,'native-real-admission-error.png')});ctx.report.emptyResultGuard.realErrorRemainsVisible=true;ctx.persist();
 await ctx.closeCurrent(page);
}
export async function restoreRegistryAcl(ctx){
 const original=ctx.registryAcl;if(!original)return;
 await ctx.command('powershell',['-NoProfile','-Command',aclPrelude+`$key=${quote(original.key)};$bytes=[Convert]::FromBase64String(${quote(original.binary)});$acl=Get-Acl -LiteralPath $key;$acl.SetSecurityDescriptorBinaryForm($bytes);Set-Acl -LiteralPath $key -AclObject $acl;$after=(Get-Acl -LiteralPath $key).GetSecurityDescriptorBinaryForm();if([Convert]::ToBase64String($after) -cne ${quote(original.binary)}){throw 'Owned registry ACL restoration differs'};'restored'`]);
 ctx.registryAcl=null;
}
export async function cleanupBoundaryDriver(ctx){
 const driver=ctx.boundaryDriver;if(!driver)return;
 const errors=[];
 if(ctx.ownedBoundaryServer)try{await stopProcess(ctx,ctx.ownedBoundaryServer);}catch(error){errors.push(error.message);}
 if(ctx.ownedBoundaryController)try{await stopProcess(ctx,ctx.ownedBoundaryController);}catch(error){errors.push(error.message);}
 if(driver.exitCode===null)driver.kill(); // Original child handle, not an arbitrary PID.
 ctx.boundaryDriver=null;
 if(errors.length)throw Error('Boundary termination unconfirmed: '+errors.join('; '));
}
