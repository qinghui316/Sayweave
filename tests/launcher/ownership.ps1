$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot '../../scripts/environment.ps1')
$launcher=Join-Path $projectRoot 'scripts/dev.ps1'
$recordPath=Join-Path $projectRoot '.local/dev/current.json'
$evidence=@()
function Assert-That($Condition,[string]$Message) { if (-not $Condition) { throw $Message }; $script:evidence+=$Message }
function Call-Launcher([string]$Action) { & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $launcher -Action $Action -Quiet; if ($LASTEXITCODE -ne 0) { throw "Launcher $Action failed" } }
$probe=Start-Process -FilePath (Join-Path $nodeDirectory 'node.exe') -ArgumentList @('-e','"setInterval(()=>{},1000)"') -WindowStyle Hidden -PassThru
$probeTime=$probe.StartTime
try {
  Call-Launcher start
  $first=Get-Content -LiteralPath $recordPath -Raw -Encoding UTF8 | ConvertFrom-Json
  $deadline=[DateTime]::UtcNow.AddSeconds(90)
  do { Start-Sleep -Milliseconds 500; $first=Get-Content -LiteralPath $recordPath -Raw -Encoding UTF8 | ConvertFrom-Json } while ($first.status -eq 'starting' -and [DateTime]::UtcNow -lt $deadline)
  Assert-That ($first.status -eq 'running') 'start reaches native readiness'
  Call-Launcher start
  $second=Get-Content -LiteralPath $recordPath -Raw -Encoding UTF8 | ConvertFrom-Json
  Assert-That ($first.pid -eq $second.pid -and $first.runId -eq $second.runId) 'duplicate start preserves owned worker'
  Call-Launcher restart
  $next=Get-Content -LiteralPath $recordPath -Raw -Encoding UTF8 | ConvertFrom-Json
  Assert-That ($next.pid -ne $first.pid -and -not (Get-Process -Id $first.pid -ErrorAction SilentlyContinue)) 'restart replaces only owned worker'
  Call-Launcher stop
  Assert-That (-not (Get-Process -Id $next.pid -ErrorAction SilentlyContinue)) 'stop ends verified worker'
  Assert-That ([bool](Get-Process -Id $probe.Id -ErrorAction SilentlyContinue)) 'unrelated Node process survives stop'
  $original=Get-Content -LiteralPath $recordPath -Raw -Encoding UTF8
  try {
    $forged=$original | ConvertFrom-Json
    $forged.pid=$probe.Id; $forged.started=$probeTime.ToUniversalTime().ToString('o')
    $forged | ConvertTo-Json | Set-Content -LiteralPath $recordPath -Encoding UTF8
    $ErrorActionPreference='Continue'
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $launcher -Action stop -Quiet *> (Join-Path $projectRoot '.local/ownership-refusal.log')
    $refusalExit=$LASTEXITCODE
    $ErrorActionPreference='Stop'
    Assert-That ($refusalExit -ne 0 -and [bool](Get-Process -Id $probe.Id -ErrorAction SilentlyContinue)) 'identity mismatch refuses stopping unrelated PID'
  } finally { $original | Set-Content -LiteralPath $recordPath -Encoding UTF8 }
  $evidence | ConvertTo-Json | Set-Content (Join-Path $projectRoot '.local/launcher-evidence.json') -Encoding UTF8
} finally {
  $still=Get-Process -Id $probe.Id -ErrorAction SilentlyContinue
  if ($still -and $still.StartTime -eq $probeTime -and $still.Path -eq (Join-Path $nodeDirectory 'node.exe')) { Stop-Process -Id $probe.Id }
}
