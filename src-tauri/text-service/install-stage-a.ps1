# Operator-only Stage A installer. Does not activate an input profile or change keyboard defaults.
[CmdletBinding()]
param(
    [ValidateSet('Install','Remove')][string]$Action = 'Install',
    [string]$PayloadDirectory = $PSScriptRoot
)
$ErrorActionPreference = 'Stop'
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = [Security.Principal.WindowsPrincipal]::new($identity)
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'This explicit component installation requires an administrator terminal. It is not run automatically.'
}
$payload = (Resolve-Path -LiteralPath $PayloadDirectory).Path
$manifest = Get-Content -LiteralPath (Join-Path $payload 'manifest.json') -Encoding UTF8 | ConvertFrom-Json
if ($manifest.schema -ne 1 -or $manifest.owner -ne 'SayweaveStageA-v1' -or $manifest.userSid -ne $identity.User.Value) {
    throw 'Payload scope or initiating user mismatch.'
}
$names = @('sayweave_text_service.dll','tsf-probe.exe')
foreach ($name in $names) {
    $expected = $manifest.hashes.$name
    if ($expected -notmatch '^[0-9A-Fa-f]{64}$' -or (Get-FileHash -LiteralPath (Join-Path $payload $name) -Algorithm SHA256).Hash -ne $expected) {
        throw "Payload hash mismatch: $name"
    }
}
$programFiles = [Environment]::GetFolderPath('ProgramFiles')
$base = [IO.Path]::GetFullPath((Join-Path $programFiles 'Sayweave\TextService'))
$target = [IO.Path]::GetFullPath((Join-Path $base 'stage-a'))
if (-not $target.StartsWith($base + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Invalid install target.' }
function Assert-Protected([string]$path) {
    if ((Get-Item -LiteralPath $path).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Install paths cannot be reparse points.' }
    $acl = Get-Acl -LiteralPath $path
    $owner = $acl.GetOwner([Security.Principal.SecurityIdentifier]).Value
    if (@('S-1-5-32-544','S-1-5-18','S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464') -notcontains $owner) { throw 'Install directory owner is not protected.' }
    $write = [Security.AccessControl.FileSystemRights]::Write -bor [Security.AccessControl.FileSystemRights]::Delete -bor [Security.AccessControl.FileSystemRights]::DeleteSubdirectoriesAndFiles -bor [Security.AccessControl.FileSystemRights]::ChangePermissions -bor [Security.AccessControl.FileSystemRights]::TakeOwnership
    foreach ($rule in $acl.GetAccessRules($true, $true, [Security.Principal.SecurityIdentifier])) {
        if ($rule.AccessControlType -ne 'Allow') { continue }
        if ($rule.PropagationFlags -band [Security.AccessControl.PropagationFlags]::InheritOnly) { continue }
        $sid = $rule.IdentityReference.Value
        if (@('S-1-5-32-544','S-1-5-18','S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464') -notcontains $sid -and ($rule.FileSystemRights -band $write) -ne 0) { throw "Unsafe writable install location: $path" }
    }
}
function Ensure-ProtectedDirectory([string]$path) {
    if (-not (Test-Path -LiteralPath $path)) {
        New-Item -ItemType Directory -Path $path | Out-Null
        $acl = [Security.AccessControl.DirectorySecurity]::new()
        $acl.SetAccessRuleProtection($true, $false)
        $admin = [Security.Principal.SecurityIdentifier]::new('S-1-5-32-544')
        $acl.SetOwner($admin)
        foreach ($sid in @('S-1-5-32-544','S-1-5-18','S-1-5-32-545')) {
            $rights = if ($sid -eq 'S-1-5-32-545') { 'ReadAndExecute' } else { 'FullControl' }
            $rule = [Security.AccessControl.FileSystemAccessRule]::new([Security.Principal.SecurityIdentifier]::new($sid), $rights, 'ContainerInherit, ObjectInherit', 'None', 'Allow')
            $acl.AddAccessRule($rule)
        }
        Set-Acl -LiteralPath $path -AclObject $acl
    }
    Assert-Protected $path
}
function Remove-EmptyTarget {
    Assert-Protected $target
    if (@(Get-ChildItem -LiteralPath $target -Force).Count -ne 0) { throw 'Unknown files remain; preserving the install directory.' }
    Remove-Item -LiteralPath $target
}
Assert-Protected $programFiles
if ($Action -eq 'Install') {
    if (Test-Path -LiteralPath $target) { throw 'Existing Stage A installation; remove it explicitly before replacing the files.' }
    Ensure-ProtectedDirectory (Join-Path $programFiles 'Sayweave')
    Ensure-ProtectedDirectory $base
    Ensure-ProtectedDirectory $target
    $registrationAttempted = $false
    try {
        foreach ($name in $names) { Copy-Item -LiteralPath (Join-Path $payload $name) -Destination (Join-Path $target $name) }
        foreach ($name in $names) {
            Assert-Protected (Join-Path $target $name)
            if ((Get-FileHash -LiteralPath (Join-Path $target $name) -Algorithm SHA256).Hash -ne $manifest.hashes.$name) { throw 'Installed file hash mismatch.' }
        }
        $registrationAttempted = $true
        & (Join-Path $target 'tsf-probe.exe') register-machine (Join-Path $target 'sayweave_text_service.dll')
        if ($LASTEXITCODE -ne 0) { throw 'Registration failed; keep the protected payload for diagnosis/removal. Do not activate the profile.' }
    } catch {
        if (-not $registrationAttempted) {
            # This fresh target was created by this invocation; no registration can reference it.
            Assert-Protected $target
            foreach ($name in $names) {
                $file = Join-Path $target $name
                if (Test-Path -LiteralPath $file) { Assert-Protected $file; Remove-Item -LiteralPath $file }
            }
            Remove-EmptyTarget
        }
        throw
    }
    Write-Output 'Stage A speech component installed. Run the probe from a normal terminal; no keyboard profile has been selected.'
} else {
    $registered = (Get-ItemProperty -LiteralPath 'Registry::HKEY_LOCAL_MACHINE\Software\Classes\CLSID\{CB473CF0-9B4C-482C-BFC2-5527D2379ACA}\InprocServer32').'(default)'
    # Canonical Rust paths may use the local Win32 verbatim prefix.
    if ($registered.StartsWith('\\?\')) { $registered = $registered.Substring(4) }
    if ($registered -ne (Join-Path $target 'sayweave_text_service.dll')) { throw 'A newer registered component exists; use its own removal entry.' }
    if (-not (Test-Path -LiteralPath $target)) { throw 'Stage A directory does not exist.' }
    Assert-Protected (Join-Path $programFiles 'Sayweave')
    Assert-Protected $base
    Assert-Protected $target
    foreach ($name in $names) {
        Assert-Protected (Join-Path $target $name)
        if ((Get-FileHash -LiteralPath (Join-Path $target $name) -Algorithm SHA256).Hash -ne $manifest.hashes.$name) { throw 'Installed version differs; refusing removal with this payload.' }
    }
    & (Join-Path $target 'tsf-probe.exe') remove-machine
    if ($LASTEXITCODE -ne 0) { throw 'Unregister failed; files remain available for retry.' }
    foreach ($name in $names) { Remove-Item -LiteralPath (Join-Path $target $name) }
    Remove-EmptyTarget
    Write-Output 'Stage A registration and exact payload removed.'
}
