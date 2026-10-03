param([ValidateSet('start','stop','restart','logs','worker')][string]$Action='start',[string]$RunId,[switch]$Quiet)
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'environment.ps1')
$devRoot=Join-Path $projectRoot '.local/dev'
New-Item -ItemType Directory -Path $devRoot -Force | Out-Null
$statePath=Join-Path $devRoot 'current.json'
function Show-Info([string]$Text) {
    Write-Host $Text
    if (-not $Quiet) { Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.MessageBox]::Show($Text,'Sayweave development') | Out-Null }
}
function Read-OwnedWorker {
    if (-not (Test-Path -LiteralPath $statePath)) { return $null }
    $record=Get-Content -LiteralPath $statePath -Raw -Encoding UTF8 | ConvertFrom-Json
    $process=Get-Process -Id $record.pid -ErrorAction SilentlyContinue
    if (-not $process) { return $null }
    $identity=Get-CimInstance Win32_Process -Filter "ProcessId=$($record.pid)"
    if ($process.StartTime.ToUniversalTime().ToString('o') -ne $record.started -or
        $identity.CommandLine -notlike "*$($record.runId)*" -or $identity.CommandLine -notlike '*dev.ps1*' -or
        $record.root -ne $projectRoot) { throw 'Existing record does not match owned worker; refusing to stop it.' }
    return $record
}
if ($Action -eq 'logs') {
    if (Test-Path -LiteralPath $statePath) { $r=Get-Content -LiteralPath $statePath -Raw -Encoding UTF8 | ConvertFrom-Json; Start-Process notepad.exe -ArgumentList @('"'+$r.log+'"'); exit }
    throw 'No development log yet. Start the development app first.'
}
if ($Action -eq 'stop' -or $Action -eq 'restart') {
    $owned=Read-OwnedWorker
    if ($owned) {
        # Killing this verified worker closes its Job handle. Windows then cleans only its owned descendants.
        Stop-Process -Id $owned.pid -Force
        for ($i=0;$i -lt 40;$i++) { if (-not (Get-Process -Id $owned.pid -ErrorAction SilentlyContinue)) { break }; Start-Sleep -Milliseconds 100 }
    }
    if ($Action -eq 'stop') { Show-Info 'Sayweave development stopped.'; exit }
    $Action='start'
}
if ($Action -eq 'start') {
    $lockPath=Join-Path $devRoot 'start.lock'
    try { $lock=[IO.File]::Open($lockPath,[IO.FileMode]::OpenOrCreate,[IO.FileAccess]::ReadWrite,[IO.FileShare]::None) } catch { Show-Info 'Another Sayweave launch is in progress.'; exit }
    try {
        $existing=Read-OwnedWorker
        if ($existing) { Show-Info ('Development is already running. Use tray to reopen Sayweave. Log: '+$existing.log); exit }
        if (-not (Test-Path (Join-Path $projectRoot 'node_modules/@tauri-apps/cli/tauri.js'))) { throw 'Dependencies missing. Run npm ci after loading scripts/environment.ps1.' }
        if (Get-NetTCPConnection -LocalPort 5173 -State Listen -ErrorAction SilentlyContinue) { throw 'Port 5173 is occupied. No process was stopped and no alternative port was selected.' }
        $run=[guid]::NewGuid().ToString('N')
        $worker=Start-Process powershell.exe -WindowStyle Hidden -PassThru -ArgumentList @('-NoProfile','-ExecutionPolicy','Bypass','-File',('"'+$PSCommandPath+'"'),'-Action','worker','-RunId',$run,'-Quiet')
        $deadline=[DateTime]::UtcNow.AddSeconds(10)
        do { Start-Sleep -Milliseconds 100; if ($worker.HasExited) { throw 'Development worker failed before initialization.' } } while (-not (Test-Path (Join-Path $devRoot ($run+'.ready'))) -and [DateTime]::UtcNow -lt $deadline)
        if (-not (Test-Path (Join-Path $devRoot ($run+'.ready')))) { throw 'Worker initialization timeout. Check .local/dev.' }
        Show-Info 'Starting Sayweave desktop development. First Rust compilation can take several minutes. Progress and failures are in .local/dev; use View Development Logs.'
    } finally { $lock.Dispose() }
    exit
}
if ($Action -eq 'worker') {
    # Process management only, not UI automation. Job lifetime owns this worker and every descendant.
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class SayweaveJob {
 [StructLayout(LayoutKind.Sequential)] struct Basic { public long p,u; public uint flags; public UIntPtr min,max; public uint active; public UIntPtr affinity; public uint priority,schedule; }
 [StructLayout(LayoutKind.Sequential)] struct Io { public ulong a,b,c,d,e,f; }
 [StructLayout(LayoutKind.Sequential)] struct Extended { public Basic basic; public Io io; public UIntPtr processMemory,jobMemory,peakProcess,peakJob; }
 [DllImport("kernel32.dll",CharSet=CharSet.Unicode)] static extern IntPtr CreateJobObject(IntPtr attrs,string name);
 [DllImport("kernel32.dll")] static extern bool SetInformationJobObject(IntPtr job,int cls,IntPtr data,uint size);
 [DllImport("kernel32.dll")] static extern bool AssignProcessToJobObject(IntPtr job,IntPtr process);
 [DllImport("kernel32.dll")] static extern IntPtr GetCurrentProcess();
 public static IntPtr Own() { IntPtr job=CreateJobObject(IntPtr.Zero,null); var e=new Extended();e.basic.flags=0x2000;int n=Marshal.SizeOf(e);IntPtr p=Marshal.AllocHGlobal(n);try { Marshal.StructureToPtr(e,p,false);if(!SetInformationJobObject(job,9,p,(uint)n)||!AssignProcessToJobObject(job,GetCurrentProcess())) throw new Exception("Cannot establish owned Windows Job");return job;}finally{Marshal.FreeHGlobal(p);} }
}
'@
    $jobHandle=[SayweaveJob]::Own()
    $log=Join-Path $devRoot ($RunId+'.log')
    $record=@{runId=$RunId;pid=$PID;started=(Get-Process -Id $PID).StartTime.ToUniversalTime().ToString('o');root=$projectRoot;log=$log;status='starting'}
    $record | ConvertTo-Json | Set-Content -LiteralPath $statePath -Encoding UTF8
    "Run $RunId`r`nTime $([DateTime]::UtcNow.ToString('o'))`r`nNode $(& node --version)`r`nRust $(& rustc --version)`r`nStage desktop:dev" | Set-Content -LiteralPath $log -Encoding UTF8
    New-Item -ItemType File -Path (Join-Path $devRoot ($RunId+'.ready')) -Force | Out-Null
    try {
        $node=Join-Path $nodeDirectory 'node.exe'
        $cli=Join-Path $projectRoot 'node_modules/@tauri-apps/cli/tauri.js'
        $ErrorActionPreference='Continue'
        & $node $cli dev 2>&1 | ForEach-Object { $line=$_.ToString(); $line | Add-Content -LiteralPath $log -Encoding UTF8; if ($line -match 'SAYWEAVE_NATIVE_READY') { $record.status='running'; $record | ConvertTo-Json | Set-Content -LiteralPath $statePath -Encoding UTF8 } }
        $exitCode=$LASTEXITCODE
        $ErrorActionPreference='Stop'
        $record.status=if ($exitCode -eq 0) { 'stopped' } else { 'failed' }
        $record.exitCode=$exitCode
        $record | ConvertTo-Json | Set-Content -LiteralPath $statePath -Encoding UTF8
        if ($exitCode -ne 0) { $Quiet=$false; Show-Info ('Desktop development failed. Log: '+$log) }
    } catch {
        $_.ToString() | Add-Content -LiteralPath $log -Encoding UTF8
        $record.status='failed';$record | ConvertTo-Json | Set-Content -LiteralPath $statePath -Encoding UTF8
        $Quiet=$false;Show-Info ('Development failed. Log: '+$log)
    }
    # Closing the process ends the Job. Do not explicitly close this handle while logging.
    [GC]::KeepAlive($jobHandle)
}
