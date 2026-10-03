param([string]$NodeVersion = '24.21.0', [string]$RustVersion = '1.99.0')
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
$toolsRoot = Join-Path $projectRoot '.local/tools'
New-Item -ItemType Directory -Force -Path $toolsRoot | Out-Null
$nodeDirectory = Join-Path $toolsRoot "node-v$NodeVersion-win-x64"
if (-not (Test-Path (Join-Path $nodeDirectory 'node.exe'))) {
    $zipName = "node-v$NodeVersion-win-x64.zip"
    $zipPath = Join-Path $toolsRoot $zipName
    $baseUrl = "https://nodejs.org/dist/v$NodeVersion"
    Invoke-WebRequest -UseBasicParsing "$baseUrl/$zipName" -OutFile $zipPath
    $hashList = (Invoke-WebRequest -UseBasicParsing "$baseUrl/SHASUMS256.txt").Content
    $expected = ($hashList -split "`n" | Where-Object { $_.Trim().EndsWith("  $zipName") }) -replace '\s+.*$', ''
    if (-not $expected -or (Get-FileHash -LiteralPath $zipPath -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expected.Trim()) { throw 'Node download checksum mismatch' }
    Expand-Archive -LiteralPath $zipPath -DestinationPath $toolsRoot -Force
}
# These locations affect only this process and its children, not other projects.
$env:PATH = "$nodeDirectory;$toolsRoot/cargo/bin;$env:PATH"
$env:CARGO_HOME = Join-Path $toolsRoot 'cargo'
$env:RUSTUP_HOME = Join-Path $toolsRoot 'rustup'
if (-not (Test-Path (Join-Path $env:CARGO_HOME 'bin/rustup.exe'))) {
    $installer = Join-Path $toolsRoot 'rustup-init.exe'
    Invoke-WebRequest -UseBasicParsing 'https://win.rustup.rs/x86_64' -OutFile $installer
    & $installer -y --no-modify-path --default-toolchain none --profile minimal
    if ($LASTEXITCODE -ne 0) { throw 'rustup setup failed' }
}
& (Join-Path $env:CARGO_HOME 'bin/rustup.exe') toolchain install $RustVersion --profile minimal --component rustfmt,clippy
if ($LASTEXITCODE -ne 0) { throw 'Rust toolchain setup failed' }
& (Join-Path $nodeDirectory 'node.exe') --version
& (Join-Path $env:CARGO_HOME 'bin/rustc.exe') "+$RustVersion" --version
