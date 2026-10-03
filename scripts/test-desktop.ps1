$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'environment.ps1')
$exe=Join-Path $projectRoot 'src-tauri/target/release/sayweave.exe'
$instances=@(Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath -eq $exe })
if ($instances.Count -gt 1) { throw 'Multiple release instances; resolve before acceptance.' }
foreach ($instance in $instances) { $process=Get-Process -Id $instance.ProcessId; if ($process.Path -eq $exe) { Stop-Process -Id $process.Id } }
$ErrorActionPreference='Continue'
& npm.cmd run desktop:build -- --features desktop-acceptance *> (Join-Path $projectRoot '.local/desktop-acceptance-build.log')
$buildExit=$LASTEXITCODE
$ErrorActionPreference='Stop'
if ($buildExit -ne 0) { throw 'Acceptance build failed. See .local/desktop-acceptance-build.log.' }
$oldFlag=$env:SAYWEAVE_DESKTOP_ACCEPTANCE
try { $env:SAYWEAVE_DESKTOP_ACCEPTANCE='1'; $app=Start-Process $exe -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $projectRoot '.local/desktop-acceptance.log') -RedirectStandardError (Join-Path $projectRoot '.local/desktop-acceptance-errors.log') } finally { $env:SAYWEAVE_DESKTOP_ACCEPTANCE=$oldFlag }
$started=$app.StartTime
$null=$app.Handle
$children=@{}
$deadline=[DateTime]::UtcNow.AddSeconds(45)
try {
  while (-not $app.WaitForExit(300) -and [DateTime]::UtcNow -lt $deadline) {
    foreach ($child in @(Get-CimInstance Win32_Process -Filter "ParentProcessId=$($app.Id)")) { $children[$child.ProcessId]=$child.CreationDate }
  }
  if (-not $app.HasExited) { throw 'Desktop acceptance timed out.' }
  $app.WaitForExit()
  $exitCode=$app.ExitCode
  $log=Get-Content (Join-Path $projectRoot '.local/desktop-acceptance.log') -Raw -Encoding UTF8
  if ($exitCode -ne 0 -or $log -notmatch 'SAYWEAVE_ACCEPTANCE_PASS' -or $log -notmatch 'SAYWEAVE_SHUTDOWN') { throw "Desktop acceptance failed (exit=$exitCode); read .local/desktop-acceptance*.log." }
  Start-Sleep -Milliseconds 600
  foreach ($id in $children.Keys) { $child=Get-Process -Id $id -ErrorAction SilentlyContinue; if ($child -and $child.StartTime -eq $children[$id]) { throw "Owned WebView child still exists after exit: $id" } }
  $log
  Write-Output 'Desktop acceptance and process cleanup passed.'
} finally {
  $remaining=Get-Process -Id $app.Id -ErrorAction SilentlyContinue
  if ($remaining -and $remaining.StartTime -eq $started -and $remaining.Path -eq $exe) { Stop-Process -Id $remaining.Id }
}
