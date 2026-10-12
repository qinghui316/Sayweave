param([switch]$CompositionOnly)
$ErrorActionPreference='Stop'
$projectRoot=Split-Path $PSScriptRoot -Parent
Set-Location -LiteralPath $projectRoot
$previous=$env:SAYWEAVE_INPUT_ACCEPTANCE
$previousPayload=$env:SAYWEAVE_TSF_PAYLOAD
$previousProxy=$env:SAYWEAVE_TSF_PROXY_PAYLOAD
try{
  $built=& node --input-type=module -e "import {buildTextService} from './scripts/build-text-service.mjs'; console.log(JSON.stringify(buildTextService(process.cwd())));"
  if($LASTEXITCODE -ne 0){throw 'Ordinary input bundle build failed.'}
  $bundle=$built[-1] | ConvertFrom-Json
  $env:SAYWEAVE_TSF_PAYLOAD=$bundle.core
  $env:SAYWEAVE_TSF_PROXY_PAYLOAD=$bundle.proxy
  & cargo build --manifest-path src-tauri/Cargo.toml --features desktop-acceptance --bin input-fixture --bin input-acceptance
  if($LASTEXITCODE -ne 0){throw 'Input fixture build failed.'}
  $env:SAYWEAVE_INPUT_ACCEPTANCE='1'
  if($CompositionOnly){
    & (Join-Path $projectRoot 'src-tauri/target/debug/input-acceptance.exe') composition
  }else{
    & (Join-Path $projectRoot 'src-tauri/target/debug/input-acceptance.exe')
  }
  if($LASTEXITCODE -ne 0){throw 'Native input acceptance failed.'}
}finally{$env:SAYWEAVE_INPUT_ACCEPTANCE=$previous;$env:SAYWEAVE_TSF_PAYLOAD=$previousPayload;$env:SAYWEAVE_TSF_PROXY_PAYLOAD=$previousProxy}
