param(
    [Parameter(Mandatory=$true)][ValidatePattern('^[a-f0-9]{40}$')][string]$ExpectedCommit,
    [Parameter(Mandatory=$true)][ValidatePattern('^[a-f0-9]{40}$')][string]$ProductCommit,
    [Parameter(Mandatory=$true)][string]$ExpectedModulePath,
    [Parameter(Mandatory=$true)][ValidatePattern('^[a-f0-9]{64}$')][string]$ExpectedComponentSha256
)
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
if ($env:GITHUB_ACTIONS -cne 'true' -or $env:RUNNER_ENVIRONMENT -cne 'github-hosted' -or $env:RUNNER_OS -cne 'Windows' -or -not [Environment]::Is64BitProcess) {
    throw 'Requires a disposable GitHub-hosted x64 Windows runner; never an owner machine.'
}
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
try {
    if (-not [Security.Principal.WindowsPrincipal]::new($identity).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { throw 'Disposable runner is not elevated.' }
} finally { $identity.Dispose() }
$actualCommit = (& git rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0 -or $actualCommit -cne $ExpectedCommit) { throw 'Inspector checkout differs from the expected exact candidate.' }
$readerPath = Join-Path $PSScriptRoot 'windows-update-metadata.cs'
if ('SayweaveMetadata.Characterization' -as [type]) { throw 'Use a fresh PowerShell process; a reader type is already loaded.' }
Add-Type -Path $readerPath
$result = [SayweaveMetadata.Characterization]::Read($ExpectedModulePath, $ExpectedComponentSha256)
$readerHash = [Security.Cryptography.SHA256]::Create()
try {
    $helperSha256 = [BitConverter]::ToString($readerHash.ComputeHash([IO.File]::ReadAllBytes($readerPath))).Replace('-', '').ToLowerInvariant()
} finally { $readerHash.Dispose() }
$receipt = [ordered]@{
    productCommit = $ProductCommit
    acceptanceCommit = $ExpectedCommit
    environment = 'GitHub-hosted disposable elevated Windows x64'
    runnerName = $env:RUNNER_NAME
    observedAt = [DateTime]::UtcNow.ToString('o')
    helperSha256 = $helperSha256
    characterization = $result
    metadataExpected = ($result.allRawExpected -and $result.allApiExpected)
}
$outputDirectory = Join-Path (Get-Location) '.local/update-metadata'
New-Item -ItemType Directory -Path $outputDirectory -Force | Out-Null
$outputPath = Join-Path $outputDirectory ([Guid]::NewGuid().ToString() + '.json')
$temporaryPath = $outputPath + '.tmp'
$json = $receipt | ConvertTo-Json -Depth 12
[IO.File]::WriteAllText($temporaryPath, $json + "`n", [Text.UTF8Encoding]::new($false))
Move-Item -LiteralPath $temporaryPath -Destination $outputPath
Write-Output $json
Write-Output "Read-only metadata receipt: $outputPath"
# Exit0 means a complete read. metadataExpected is the separate correctness result.
