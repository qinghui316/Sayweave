param([switch]$CompositionOnly)
$ErrorActionPreference='Stop'
$projectRoot=Split-Path $PSScriptRoot -Parent
Set-Location -LiteralPath $projectRoot
$previous=$env:SAYWEAVE_INPUT_ACCEPTANCE
$previousPayload=$env:SAYWEAVE_TSF_PAYLOAD
try{
  if(-not $env:SAYWEAVE_TSF_PAYLOAD){
    $env:SAYWEAVE_TSF_PAYLOAD=Join-Path $projectRoot 'src-tauri/text-service/target/release/sayweave_text_service.dll'
  }
  if(-not (Test-Path -LiteralPath $env:SAYWEAVE_TSF_PAYLOAD -PathType Leaf)){throw 'Ordinary component payload missing. Build the development program first.'}
  & cargo build --manifest-path src-tauri/Cargo.toml --features desktop-acceptance --bin input-fixture --bin input-acceptance
  if($LASTEXITCODE -ne 0){throw 'Input fixture build failed.'}
  $env:SAYWEAVE_INPUT_ACCEPTANCE='1'
  if($CompositionOnly){
    & (Join-Path $projectRoot 'src-tauri/target/debug/input-acceptance.exe') composition
  }else{
    & (Join-Path $projectRoot 'src-tauri/target/debug/input-acceptance.exe')
  }
  if($LASTEXITCODE -ne 0){throw 'Native input acceptance failed.'}
}finally{$env:SAYWEAVE_INPUT_ACCEPTANCE=$previous;$env:SAYWEAVE_TSF_PAYLOAD=$previousPayload}
