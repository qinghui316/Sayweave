$ErrorActionPreference = 'Stop'
$probe = Join-Path $PSScriptRoot 'target\debug\tsf-probe.exe'
& $probe fixture
if ($LASTEXITCODE -ne 0) { throw 'Owned TSF input assertions failed.' }
