param([switch]$Autostart)
$ErrorActionPreference='Stop'
$projectRoot=Split-Path $PSScriptRoot -Parent
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
$OutputEncoding=[Text.UTF8Encoding]::new($false)
Set-Location -LiteralPath $projectRoot
$null=New-Item -ItemType Directory -Force -Path (Join-Path $projectRoot '.local')
$exe=Join-Path $projectRoot 'src-tauri/target/release/desktop-acceptance.exe'
$instances=@(Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath -eq $exe })
if ($instances.Count -gt 0) { throw 'An existing release instance is running. Save and exit it before acceptance; this test only owns the instance it starts.' }
$ErrorActionPreference='Continue'
& npm.cmd run build *> (Join-Path $projectRoot '.local/desktop-acceptance-build.log')
$buildExit=$LASTEXITCODE
if($buildExit -eq 0){
  & cargo build --manifest-path src-tauri/Cargo.toml --release --features desktop-acceptance,tauri/custom-protocol --bin desktop-acceptance *>> (Join-Path $projectRoot '.local/desktop-acceptance-build.log')
}
$buildExit=$LASTEXITCODE
$ErrorActionPreference='Stop'
if ($buildExit -ne 0) { throw 'Acceptance build failed. See .local/desktop-acceptance-build.log.' }
$oldFlag=$env:SAYWEAVE_DESKTOP_ACCEPTANCE
$oldDataId=$env:SAYWEAVE_DATA_ACCEPTANCE_ID
$acceptanceId=[guid]::NewGuid().ToString()
& node scripts/storage.mjs begin-desktop $acceptanceId | Out-Null
if($LASTEXITCODE -ne 0){throw 'Storage reservation failed before desktop launch.'}
$env:SAYWEAVE_DATA_ACCEPTANCE_ID=$acceptanceId
try {
  $env:SAYWEAVE_DESKTOP_ACCEPTANCE='1'
  $start=@{FilePath=$exe;WindowStyle='Hidden';PassThru=$true;RedirectStandardOutput=(Join-Path $projectRoot '.local/desktop-acceptance.log');RedirectStandardError=(Join-Path $projectRoot '.local/desktop-acceptance-errors.log')}
  if($Autostart){$start.ArgumentList=@('--autostart')}
  $app=Start-Process @start
} finally { $env:SAYWEAVE_DESKTOP_ACCEPTANCE=$oldFlag; $env:SAYWEAVE_DATA_ACCEPTANCE_ID=$oldDataId }
$started=$app.StartTime
$null=$app.Handle
$children=@{}
$deadline=[DateTime]::UtcNow.AddSeconds(45)
$acceptancePassed=$false
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
  $acceptancePassed=$true
} finally {
  $remaining=Get-Process -Id $app.Id -ErrorAction SilentlyContinue
  if ($remaining -and $remaining.StartTime -eq $started -and $remaining.Path -eq $exe) { Stop-Process -Id $remaining.Id }
  $storageResult=if($acceptancePassed){'passed'}else{'failed'}
  & node scripts/storage.mjs finish-desktop $acceptanceId $storageResult
  if($LASTEXITCODE -ne 0){throw 'Validation storage settlement failed; inspect the retained context.'}
}
