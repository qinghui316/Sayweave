$ErrorActionPreference='Stop'
Set-Location -LiteralPath (Split-Path $PSScriptRoot -Parent)
& node scripts/test-windows-update.mjs @args
if($LASTEXITCODE -ne 0){throw 'Windows update acceptance failed; see .local/windows-update-acceptance.json.'}
