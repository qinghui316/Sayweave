$ErrorActionPreference = 'Stop'
$probe = Join-Path $PSScriptRoot 'target\debug\tsf-probe.exe'
& $probe fixture
if ($LASTEXITCODE -ne 0) { throw 'Owned TSF input assertions failed.' }
& $probe fixture (Join-Path $PSScriptRoot 'target\debug\sayweave_text_service.dll') plain
if ($LASTEXITCODE -ne 0) { throw 'Owned legacy EDIT composition/final text assertions failed.' }
