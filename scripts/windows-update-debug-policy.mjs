// WebView2 150+ ignores environment overrides in an elevated host. This lease
// belongs only to a disposable acceptance runner, never the shipped application.
import {execFileSync} from 'node:child_process';
import {randomUUID} from 'node:crypto';

const keys=['Software\\Policies\\Microsoft\\Edge','Software\\Policies\\Microsoft\\Edge\\WebView2','Software\\Policies\\Microsoft\\Edge\\WebView2\\AdditionalBrowserArguments'];
export function validateDebugPolicyState(state){
 if(!state||!Array.isArray(state.present)||state.present.length!==keys.length||state.present.some(value=>typeof value!=='boolean')||typeof state.appPresent!=='boolean'||typeof state.wildcardPresent!=='boolean'||!Number.isInteger(state.running)||state.running<0)throw Error('Incomplete disposable debug policy prestate');
 if(state.appPresent||state.wildcardPresent||state.running)throw Error('Disposable debug policy conflicts with existing policy or running application');
 if(state.present.some((value,index)=>value&&state.present.slice(0,index).some(parent=>!parent)))throw Error('Inconsistent disposable debug policy parents');
 return state;
}
export function debugPolicyScript(operation,port,initial=null,leaseId=null,requireValue=false){
 if(!['read','install','remove'].includes(operation)||!Number.isInteger(port)||port<1024||port>65535)throw Error('Invalid disposable debug policy operation');
 if(operation!=='read'){validateDebugPolicyState(initial);if(typeof leaseId!=='string'||!/^([a-f0-9]{8}-)([a-f0-9]{4}-){3}[a-f0-9]{12}$/.test(leaseId))throw Error('Unique disposable debug lease required');}
 const flags=`--remote-debugging-port=${port} --remote-debugging-address=127.0.0.1 --sayweave-acceptance-lease=${leaseId}`;
 const pre=initial?.present??[];
 return String.raw`$ErrorActionPreference='Stop';[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false);
$principal=New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent());if(-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)){throw 'Disposable policy requires elevated runner'};
$keys=@('Software\Policies\Microsoft\Edge','Software\Policies\Microsoft\Edge\WebView2','Software\Policies\Microsoft\Edge\WebView2\AdditionalBrowserArguments');$machine=[Microsoft.Win32.Registry]::LocalMachine;$name='sayweave.exe';$flags='${flags}';
function Read-State {$present=@();foreach($path in $keys){$item=$machine.OpenSubKey($path);$present+=($null -ne $item);if($item){$item.Dispose()}};$item=$machine.OpenSubKey($keys[-1]);$names=@();if($item){$names=@($item.GetValueNames());$item.Dispose()};return @{present=$present;appPresent=($names -contains $name);wildcardPresent=($names -contains '*');running=@(Get-Process -Name sayweave -ErrorAction SilentlyContinue).Count}}
$operation='${operation}';$initial=@(${pre.map(value=>value?'$true':'$false').join(',')});
if($operation -eq 'read'){ConvertTo-Json -InputObject (Read-State) -Compress;exit};
if($operation -eq 'install'){
 $state=Read-State;if($state.appPresent -or $state.wildcardPresent -or $state.running -ne 0 -or ($state.present -join ',') -ne ($initial -join ',')){throw 'Disposable policy prestate changed'};
 $item=$machine.CreateSubKey($keys[-1]);try{if($item.GetValueNames() -contains $name -or $item.GetValueNames() -contains '*'){throw 'Disposable policy ownership changed'};$item.SetValue($name,$flags,[Microsoft.Win32.RegistryValueKind]::String);$item.Flush();if($item.GetValueKind($name) -ne [Microsoft.Win32.RegistryValueKind]::String -or $item.GetValue($name) -cne $flags){throw 'Disposable policy write unconfirmed'}}finally{$item.Dispose()};
 ConvertTo-Json -InputObject @{installed=$true;port=${port};loopback=$true} -Compress;exit
};
$item=$machine.OpenSubKey($keys[-1],$true);if($item){try{if($item.GetValueNames() -contains $name){if($item.GetValueKind($name) -ne [Microsoft.Win32.RegistryValueKind]::String -or $item.GetValue($name) -cne $flags){throw 'Disposable policy value changed; refusing deletion'};$item.DeleteValue($name);$item.Flush();if($item.GetValueNames() -contains $name){throw 'Disposable policy removal unconfirmed'}}elseif(${requireValue?'$true':'$false'}){throw 'Installed disposable policy disappeared'}}finally{$item.Dispose()}}elseif(${requireValue?'$true':'$false'}){throw 'Installed disposable policy key disappeared'};
$state=Read-State;if($state.appPresent){throw 'Disposable policy value removal not confirmed'};$retained=@();for($n=0;$n -lt $keys.Count;$n++){if(-not $initial[$n] -and $state.present[$n]){$retained+=($keys[$n])}};ConvertTo-Json -InputObject @{removed=$true;ownValueAbsent=$true;retainedCreatedKeys=$retained} -Compress`;
}
export function prepareDisposableDebugPolicy(port){
 if(process.platform!=='win32'||process.env.GITHUB_ACTIONS!=='true'&&process.env.SAYWEAVE_DISPOSABLE_WINDOWS!=='1')throw Error('Debug policy is restricted to a disposable Windows runner');
 const run=script=>JSON.parse(execFileSync('powershell',['-NoProfile','-Command',script],{encoding:'utf8',windowsHide:true,timeout:10000,maxBuffer:65536}));
 const initial=validateDebugPolicyState(run(debugPolicyScript('read',port)));
 const receipt={kind:'temporary-disposable-HKLM-WebView-policy',leaseId:randomUUID(),port,loopback:true,initial,attempted:false,installed:false,removed:false};
 // Return ownership before changing anything, so even partial failure is cleaned.
 return {receipt,install(){if(receipt.attempted)throw Error('Disposable debug policy already attempted');receipt.attempted=true;const result=run(debugPolicyScript('install',port,initial,receipt.leaseId));if(result.installed!==true)throw Error('Disposable debug policy not confirmed');receipt.installed=true;},remove(){if(receipt.removed)return;if(receipt.attempted){const result=run(debugPolicyScript('remove',port,initial,receipt.leaseId,receipt.installed));if(result.removed!==true||result.ownValueAbsent!==true)throw Error('Disposable debug policy cleanup unconfirmed');receipt.retainedCreatedKeys=result.retainedCreatedKeys;}receipt.removed=true;}};
}
