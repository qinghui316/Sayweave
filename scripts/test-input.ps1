$ErrorActionPreference='Stop'
$projectRoot=Split-Path $PSScriptRoot -Parent
Set-Location -LiteralPath $projectRoot
& cargo build --manifest-path src-tauri/Cargo.toml --features desktop-acceptance --bin input-fixture --bin input-acceptance
if($LASTEXITCODE -ne 0){throw 'Input fixture build failed.'}
$previous=$env:SAYWEAVE_INPUT_ACCEPTANCE
try{
  $env:SAYWEAVE_INPUT_ACCEPTANCE='1'
  & (Join-Path $projectRoot 'src-tauri/target/debug/input-acceptance.exe')
  if($LASTEXITCODE -ne 0){throw 'Native input acceptance failed.'}
}finally{$env:SAYWEAVE_INPUT_ACCEPTANCE=$previous}
