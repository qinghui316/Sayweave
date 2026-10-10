import {spawn,execFileSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {mkdirSync,readFileSync,writeFileSync,existsSync,lstatSync,openSync,readSync,closeSync} from 'node:fs';
import path from 'node:path';

const limit=65536;
const delay=ms=>new Promise(resolve=>setTimeout(resolve,ms));
export function boundedTail(previous,bytes){return Buffer.concat([previous,Buffer.from(bytes)]).subarray(-limit);}
export function safeStartupText(value,secrets=[]){
 let text=String(value);
 for(const secret of secrets.filter(s=>typeof s==='string'&&s.length>=4))text=text.replaceAll(secret,'[redacted]');
 return text.replace(/(Bearer\s+|(?:token|api[_-]?key|password)\s*[=:]\s*)[^\s,;]+/gi,'$1[redacted]');
}
export function pageReady(text,pid,createdAt=0){
 const threshold=typeof createdAt==='number'?createdAt:Date.parse(createdAt);
 return Number.isFinite(threshold)&&[...text.matchAll(/operation=startup phase=main_page_ready pid=(\d+) at_ms=(\d+) /g)].some(match=>Number(match[1])===pid&&Number(match[2])>=threshold&&Number(match[2])<=Date.now()+1000);
}
export function validateEndpoint(endpoint,port){
 if(typeof endpoint?.webSocketDebuggerUrl!=='string')throw Error('WebView endpoint has no browser identity');
 const url=new URL(endpoint.webSocketDebuggerUrl);
 if(url.protocol!=='ws:'||!['127.0.0.1','localhost'].includes(url.hostname)||url.port!==String(port)||!url.pathname.startsWith('/devtools/browser/'))throw Error('WebView endpoint escaped owned loopback port');
 return endpoint.webSocketDebuggerUrl;
}
function tail(file){
 if(!existsSync(file))return '';
 const info=lstatSync(file);if(!info.isFile()||info.isSymbolicLink())throw Error('Startup log is not an owned regular file');
 const fd=openSync(file,'r');try{const buffer=Buffer.alloc(Math.min(limit,info.size));const count=readSync(fd,buffer,0,buffer.length,Math.max(0,info.size-buffer.length));return buffer.subarray(0,count).toString('utf8');}finally{closeSync(fd);}
}
export function installedState(exe,port=9291){
 if(!Number.isInteger(port)||port<1024||port>65535)throw Error('Invalid owned debug port');
 const literal="'"+exe.replaceAll("'","''")+"'";
 // Enumerate only windows of the exact installed executable and its verified
 // descendants. Do not retain unrelated command lines or desktop titles.
 const script=String.raw`$ErrorActionPreference='Stop';[Console]::OutputEncoding=[System.Text.UTF8Encoding]::new($false);Add-Type -TypeDefinition @'
using System;using System.Collections.Generic;using System.Runtime.InteropServices;using System.Text;
public class OwnedStartupWindows {
 public delegate bool Callback(IntPtr h,IntPtr p);
 [DllImport("user32.dll")]static extern bool EnumWindows(Callback c,IntPtr p);
 [DllImport("user32.dll")]static extern uint GetWindowThreadProcessId(IntPtr h,out uint p);
 [DllImport("user32.dll")]static extern bool IsWindowVisible(IntPtr h);
 [DllImport("user32.dll",CharSet=CharSet.Unicode)]static extern int GetClassName(IntPtr h,StringBuilder s,int n);
 [StructLayout(LayoutKind.Sequential)]public struct Rect{public int left,top,right,bottom;}
 [DllImport("user32.dll")]static extern bool GetWindowRect(IntPtr h,out Rect r);
 public static List<object> Read(int pid){var r=new List<object>();EnumWindows((h,p)=>{uint id;GetWindowThreadProcessId(h,out id);if(id==pid){var s=new StringBuilder(128);GetClassName(h,s,128);Rect rect;GetWindowRect(h,out rect);r.Add(new {handle=h.ToInt64().ToString(),visible=IsWindowVisible(h),windowClass=s.ToString(),width=rect.right-rect.left,height=rect.bottom-rect.top});}return true;},IntPtr.Zero);return r;}
}
'@;$all=@(Get-CimInstance Win32_Process);$parents=@($all|Where-Object {$_.ExecutablePath -eq ${literal}});$tree=@($parents);for($n=0;$n -lt $tree.Count;$n++){if($tree.Count -gt 32){throw 'Owned WebView tree exceeded bound'};$parent=$tree[$n];foreach($child in @($all|Where-Object {$_.ParentProcessId -eq $parent.ProcessId -and $_.CreationDate -ge $parent.CreationDate})){if(-not @($tree|Where-Object {$_.ProcessId -eq $child.ProcessId}).Count){$tree+=@($child)}}};$apps=@($parents|ForEach-Object {@{ProcessId=$_.ProcessId;CreationDate=$_.CreationDate.ToUniversalTime().ToString('o');ExecutablePath=$_.ExecutablePath;windows=@([OwnedStartupWindows]::Read($_.ProcessId))}});$webviews=@($tree|Where-Object {$_.Name -eq 'msedgewebview2.exe'}|ForEach-Object {@{ProcessId=$_.ProcessId;ParentProcessId=$_.ParentProcessId;CreationDate=$_.CreationDate.ToUniversalTime().ToString('o');ExecutablePath=$_.ExecutablePath;debugPort=if($_.CommandLine -match '--remote-debugging-port=(\d+)'){$Matches[1]}else{$null};browser=($_.CommandLine -notmatch '--type=')}});$listeners=@(Get-NetTCPConnection -State Listen -LocalPort ${port} -ErrorAction SilentlyContinue | Select-Object LocalAddress,LocalPort,OwningProcess);ConvertTo-Json -InputObject @{applications=$apps;webviews=$webviews;listeners=$listeners} -Depth 6 -Compress`;
 return JSON.parse(execFileSync('powershell',['-NoProfile','-Command',script],{encoding:'utf8',windowsHide:true,timeout:10000,maxBuffer:128*1024}));
}
async function endpointAt(port){
 const response=await fetch(`http://127.0.0.1:${port}/json/version`,{signal:AbortSignal.timeout(1000)});
 if(!response.ok)throw Error(`WebView HTTP ${response.status}`);
 let bytes=0,parts=[];for await(const part of response.body){bytes+=part.length;if(bytes>limit)throw Error('WebView metadata exceeded bound');parts.push(part);}
 const endpoint=JSON.parse(Buffer.concat(parts).toString('utf8'));validateEndpoint(endpoint,port);return endpoint;
}

export async function installedStartup({exe,dataRoot,directory,port,launch=true,previous,record,persist,onOwned,onSpawn,timeoutMs=60000,allowMissingPageMarker=false,io={}}){
 mkdirSync(directory,{recursive:true});
 const secrets=[process.env.GH_TOKEN,process.env.GITHUB_TOKEN,process.env.TAURI_SIGNING_PRIVATE_KEY,process.env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD];
 const entry={exe,sha256:null,startedAt:new Date().toISOString(),launch,phase:'preflight',passed:false};record.push(entry);
 const prefix=`startup-${record.length}`,logFile=path.join(dataRoot,'logs','sayweave.log');
 let child,closed=false,stdout=Buffer.alloc(0),stderr=Buffer.alloc(0);
 const inspect=io.inspect??installedState,launchProcess=io.spawn??spawn,occupied=io.occupied??(()=>fetch(`http://127.0.0.1:${port}/json/version`,{signal:AbortSignal.timeout(1000)}).then(()=>true).catch(()=>false));
 function save(){
  entry.elapsedMs=Date.now()-started;
  writeFileSync(path.join(directory,`${prefix}.stdout.log`),safeStartupText(stdout.toString('utf8'),secrets));
  writeFileSync(path.join(directory,`${prefix}.stderr.log`),safeStartupText(stderr.toString('utf8'),secrets));
  const native=tail(logFile);writeFileSync(path.join(directory,`${prefix}.native.log`),safeStartupText(native,secrets));
  entry.logs={stdout:`${prefix}.stdout.log`,stderr:`${prefix}.stderr.log`,native:`${prefix}.native.log`,maxBytes:limit};persist();return native;
 }
 const started=Date.now();
 try{
  entry.initial=inspect(exe,port);save();
  if(launch){
   entry.sha256=createHash('sha256').update(readFileSync(exe)).digest('hex');
   if(entry.initial.applications.length)throw Error('Installed executable already has a process; refusing unowned reuse');
   if(entry.initial.listeners?.length)throw Error('Owned WebView listener occupied');
   if(await occupied())throw Error('Owned WebView port occupied');
   child=launchProcess(exe,[],{windowsHide:true,env:{...process.env,WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:`--remote-debugging-port=${port} --remote-debugging-address=127.0.0.1`},stdio:['ignore','pipe','pipe']});
   onSpawn(child);entry.pid=child.pid??null;
   child.stdout.on('data',bytes=>{stdout=boundedTail(stdout,bytes);});child.stderr.on('data',bytes=>{stderr=boundedTail(stderr,bytes);});
   child.once('error',error=>{entry.spawnError={code:error.code??null,message:safeStartupText(error.message,secrets).slice(0,1024)};});
   child.once('close',(code,signal)=>{closed=true;entry.exitCode=code;entry.signal=signal;save();});
  }
  entry.phase='process';let owned;
  while(Date.now()-started<timeoutMs){
   if(closed||entry.spawnError)throw Error(`Installed process ended during ${entry.phase}: exit=${entry.exitCode??'none'} ${entry.spawnError?.code??''}`);
   entry.last=inspect(exe,port);const apps=entry.last.applications;
   if(apps.length>1)throw Error('Installed process identity is not unique');
   const current=apps[0];
   if(current){
    if(launch&&current.ProcessId!==child.pid)throw Error('Installed process differs from spawned PID');
    if(!launch&&previous?.ProcessId===current.ProcessId&&previous?.CreationDate===current.CreationDate){entry.phase='waitingForNewProcess';save();await delay(500);continue;}
    if(owned&&(owned.ProcessId!==current.ProcessId||owned.CreationDate!==current.CreationDate))throw Error('Owned application changed during startup');
    owned=current;entry.process=owned;onOwned(owned);
    if(!entry.sha256){try{entry.sha256=createHash('sha256').update(readFileSync(exe)).digest('hex');}catch(error){entry.executableReadError=error.code??String(error.message).slice(0,1024);entry.phase='executable';save();await delay(500);continue;}}
    const native=save();
    entry.processReady=true;entry.windowReady=current.windows.some(w=>w.visible&&w.windowClass==='Tauri Window'&&w.width>=640&&w.height>=400);
    entry.pageReady=pageReady(native+'\n'+stdout.toString('utf8')+'\n'+stderr.toString('utf8'),current.ProcessId,current.CreationDate);
    entry.legacyPageProbe=allowMissingPageMarker;
    entry.phase=!entry.windowReady?'window':!entry.pageReady&&!allowMissingPageMarker?'page':'debugEndpoint';
    if(entry.windowReady&&(entry.pageReady||allowMissingPageMarker)){
     try{
      const listeners=entry.last.listeners??[];
      if(!listeners.length||!listeners.every(listener=>['127.0.0.1','::1'].includes(listener.LocalAddress)&&entry.last.webviews.some(p=>p.ProcessId===listener.OwningProcess&&p.browser&&p.debugPort===String(port))))throw Error('Debug listener is not the owned loopback WebView browser');
      const endpoint=await endpointAt(port);
      entry.endpoint=endpoint;entry.phase='debugEndpointReady';entry.passed=true;save();return {child,owned,endpoint,entry};
     }catch(error){entry.endpointError=String(error.message).slice(0,1024);}
    }
   }else save();
   await delay(500);
  }
  throw Error(`Installed startup timed out at ${entry.phase}`);
 }catch(error){entry.error=safeStartupText(error.message,secrets).slice(0,2048);save();throw error;}
}
