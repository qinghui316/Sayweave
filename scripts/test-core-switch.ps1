$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'environment.ps1')
$proofRoot=Join-Path $projectRoot '.local/core-switch/current'
New-Item -ItemType Directory -Force -Path $proofRoot | Out-Null
$before=$env:SAYWEAVE_ENGINE_BUILD_ID
try {
  & cargo build --locked --manifest-path src-tauri/text-service/Cargo.toml --features acceptance --bin tsf-probe
  if($LASTEXITCODE -ne 0){throw 'Core switch probe build failed.'}
  foreach($entry in @(@('A','1'),@('B','2'),@('C','3'))){
    $env:SAYWEAVE_ENGINE_BUILD_ID=$entry[1]
    & cargo build --locked --manifest-path src-tauri/text-service/Cargo.toml --features engine --lib
    if($LASTEXITCODE -ne 0){throw 'Core switch engine build failed.'}
    $directory=Join-Path $proofRoot $entry[0]
    New-Item -ItemType Directory -Force -Path $directory | Out-Null
    Copy-Item -LiteralPath 'src-tauri/text-service/target/debug/sayweave_text_service.dll' -Destination $directory
  }
  $env:SAYWEAVE_ENGINE_BUILD_ID='0'
  & cargo build --locked --manifest-path src-tauri/text-service/Cargo.toml --no-default-features --features proxy,acceptance --lib
  if($LASTEXITCODE -ne 0){throw 'Core switch proxy build failed.'}
  $proxy=Join-Path $proofRoot 'proxy.dll'
  Copy-Item -LiteralPath 'src-tauri/text-service/target/debug/sayweave_text_service.dll' -Destination $proxy
  $probe=Join-Path $projectRoot 'src-tauri/text-service/target/debug/tsf-probe.exe'
  & $probe core-switch $proxy "$proofRoot/A/sayweave_text_service.dll" "$proofRoot/B/sayweave_text_service.dll" "$proofRoot/C/sayweave_text_service.dll" 2>&1 | Tee-Object -FilePath "$proofRoot/body-receipts.log"
  if($LASTEXITCODE -ne 0){throw 'Core switch same-process gate failed.'}
  & $probe core-switch-multi $proxy "$proofRoot/A/sayweave_text_service.dll" "$proofRoot/B/sayweave_text_service.dll" 2>&1 | Tee-Object -FilePath "$proofRoot/multi-sta.log"
  if($LASTEXITCODE -ne 0){throw 'Core switch STA isolation failed.'}
} finally {$env:SAYWEAVE_ENGINE_BUILD_ID=$before}
