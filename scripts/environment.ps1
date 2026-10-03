$projectRoot = Split-Path $PSScriptRoot -Parent
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
$OutputEncoding = [Text.UTF8Encoding]::new($false)
$toolsRoot = Join-Path $projectRoot '.local/tools'
$nodeDirectory = Join-Path $toolsRoot 'node-v24.21.0-win-x64'
if (-not (Test-Path (Join-Path $nodeDirectory 'node.exe'))) { throw 'Project Node missing. Run scripts/setup-tools.ps1 first.' }
$env:PATH = "$nodeDirectory;$toolsRoot/cargo/bin;$env:PATH"
$env:CARGO_HOME = Join-Path $toolsRoot 'cargo'
$env:RUSTUP_HOME = Join-Path $toolsRoot 'rustup'
Set-Location -LiteralPath $projectRoot
