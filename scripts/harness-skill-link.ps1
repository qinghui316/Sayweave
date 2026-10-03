# ECL-HARNESS-CONNECTOR
param([switch] $Detach)

$ErrorActionPreference = "Stop"

$SkillName = "beaver-4a20fc43577a-harness"
$ProjectId = "beaver-4a20fc43577a"

function Invoke-Git([string[]] $Arguments, [string] $WorkingDirectory) {
    $result = & git -C $WorkingDirectory @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "git command failed: git -C $WorkingDirectory $($Arguments -join ' ')"
    }
    return ($result | Out-String).Trim()
}

function Resolve-LinkTarget([string] $Path) {
    $item = Get-Item -LiteralPath $Path -Force
    if ($item.LinkType -and $item.Target) {
        $target = @($item.Target)[0]
        if (-not [System.IO.Path]::IsPathRooted($target)) {
            $target = Join-Path $item.Parent.FullName $target
        }
        return [System.IO.Path]::GetFullPath($target).TrimEnd('\', '/')
    }
    return [System.IO.Path]::GetFullPath($item.FullName).TrimEnd('\', '/')
}

function Assert-PhysicalAncestors([string] $Root, [string] $Path) {
    $normalizedRoot = [System.IO.Path]::GetFullPath($Root).TrimEnd('\', '/')
    $normalizedPath = [System.IO.Path]::GetFullPath($Path).TrimEnd('\', '/')
    $prefix = $normalizedRoot + [System.IO.Path]::DirectorySeparatorChar
    if (-not $normalizedPath.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Skill path escapes this worktree: $Path"
    }
    $current = Split-Path -Parent $normalizedPath
    while ($current -and $current -ine $normalizedRoot) {
        if (Test-Path -LiteralPath $current) {
            $item = Get-Item -LiteralPath $current -Force
            if ($item.LinkType) {
                throw "Skill path must not traverse a link or junction: $current"
            }
        }
        $parent = Split-Path -Parent $current
        if ($parent -eq $current) { throw "Skill path escapes this worktree: $Path" }
        $current = $parent
    }
}

function Add-SkillLink([string] $Root, [string] $Path, [string] $Target) {
    $normalizedPath = [System.IO.Path]::GetFullPath($Path).TrimEnd('\', '/')
    $normalizedTarget = [System.IO.Path]::GetFullPath($Target).TrimEnd('\', '/')
    if ($normalizedPath -ieq $normalizedTarget) {
        return "physical"
    }
    Assert-PhysicalAncestors $Root $Path
    if (Test-Path -LiteralPath $Path) {
        if ((Resolve-LinkTarget $Path) -ieq $normalizedTarget) {
            return "existing"
        }
        throw "Skill path collision: $Path"
    }
    New-Item -ItemType Directory -Path (Split-Path -Parent $Path) -Force | Out-Null
    $output = & cmd /c mklink /J $Path $Target 2>&1
    if ($LASTEXITCODE -ne 0) {
        throw (($output | Out-String).Trim())
    }
    return "attached"
}

function Add-LocalSkillExcludes([string] $CommonDir) {
    $exclude = Join-Path $CommonDir "info/exclude"
    $lines = if (Test-Path -LiteralPath $exclude -PathType Leaf) {
        @(Get-Content -LiteralPath $exclude -Encoding UTF8)
    } else {
        @()
    }
    $wanted = @(
        "/.agents/skills/$SkillName",
        "/.claude/skills/$SkillName"
    )
    $changed = $false
    foreach ($value in $wanted) {
        if ($lines -notcontains $value) {
            $lines += $value
            $changed = $true
        }
    }
    if ($changed) {
        [System.IO.Directory]::CreateDirectory((Split-Path -Parent $exclude)) | Out-Null
        $encoding = New-Object System.Text.UTF8Encoding($false)
        [System.IO.File]::WriteAllText($exclude, (($lines -join "`n").TrimEnd() + "`n"), $encoding)
    }
}

$current = (Get-Location).Path
$root = Invoke-Git @("rev-parse", "--show-toplevel") $current
$commonRaw = Invoke-Git @("rev-parse", "--git-common-dir") $root
$common = if ([System.IO.Path]::IsPathRooted($commonRaw)) {
    [System.IO.Path]::GetFullPath($commonRaw)
} else {
    [System.IO.Path]::GetFullPath((Join-Path $root $commonRaw))
}
$primary = if ((Split-Path -Leaf $common) -eq ".git") {
    Split-Path -Parent $common
} else {
    $line = (Invoke-Git @("worktree", "list", "--porcelain") $root).Split("`n") |
        Where-Object { $_.StartsWith("worktree ") } |
        Select-Object -First 1
    if (-not $line) { throw "could not resolve the primary worktree" }
    $line.Substring("worktree ".Length).Trim()
}
$canonical = Join-Path $primary ".agents/skills/$SkillName"
$links = [ordered]@{
    codex = Join-Path $root ".agents/skills/$SkillName"
    claude = Join-Path $root ".claude/skills/$SkillName"
}

if ($Detach) {
    if ([System.IO.Path]::GetFullPath($root).TrimEnd('\', '/') -ieq
        [System.IO.Path]::GetFullPath($primary).TrimEnd('\', '/')) {
        throw "the primary worktree hosts the physical project Harness and cannot be detached"
    }
    $result = [ordered]@{}
    foreach ($entry in $links.GetEnumerator()) {
        $item = Get-Item -LiteralPath $entry.Value -Force -ErrorAction SilentlyContinue
        if (-not $item) {
            $result[$entry.Key] = [ordered]@{ path = $entry.Value; status = "missing" }
        } elseif (-not $item.LinkType) {
            throw "refusing to detach an unmanaged physical Skill path: $($entry.Value)"
        } elseif ((Resolve-LinkTarget $entry.Value) -ine
            [System.IO.Path]::GetFullPath($canonical).TrimEnd('\', '/')) {
            throw "refusing to detach a Skill link with the wrong target: $($entry.Value)"
        } else {
            $result[$entry.Key] = [ordered]@{ path = $entry.Value; status = "detached" }
        }
    }
    $removed = [System.Collections.Generic.List[string]]::new()
    try {
        foreach ($entry in $links.GetEnumerator()) {
            if ($result[$entry.Key].status -eq "detached") {
                [System.IO.Directory]::Delete($entry.Value)
                $removed.Add($entry.Value)
            }
        }
    } catch {
        $detachError = $_
        $rollbackErrors = [System.Collections.Generic.List[string]]::new()
        for ($index = $removed.Count - 1; $index -ge 0; $index--) {
            try {
                Add-SkillLink $root $removed[$index] $canonical | Out-Null
            } catch {
                $rollbackErrors.Add("$($removed[$index]): $($_.Exception.Message)")
            }
        }
        $detail = if ($rollbackErrors.Count) { "; rollback failed for " + ($rollbackErrors -join ", ") } else { "" }
        throw "could not detach all shared Harness links: $($detachError.Exception.Message)$detail"
    }
    [ordered]@{ ok = $true; action = "detached"; skill = $canonical; links = $result } |
        ConvertTo-Json -Depth 5
    exit 0
}

$canonicalItem = Get-Item -LiteralPath $canonical -Force -ErrorAction SilentlyContinue
if ($canonicalItem -and $canonicalItem.LinkType) {
    throw "canonical project Harness must be physical: $canonical"
}
if (-not $canonicalItem -or -not (Test-Path -LiteralPath (Join-Path $canonical "SKILL.md") -PathType Leaf)) {
    throw "canonical project Harness is missing: $canonical"
}
$manifest = Get-Content -Raw -Encoding UTF8 (Join-Path $canonical "state/manifest.json") | ConvertFrom-Json
if ($manifest.project_id -ne $ProjectId -or $manifest.skill_name -ne $SkillName) {
    throw "canonical project Harness manifest does not match this Git project"
}
Add-LocalSkillExcludes $common

$result = [ordered]@{}
$created = [System.Collections.Generic.List[string]]::new()
try {
    foreach ($entry in $links.GetEnumerator()) {
        $status = Add-SkillLink $root $entry.Value $canonical
        $result[$entry.Key] = [ordered]@{
            path = $entry.Value
            status = $status
        }
        if ($status -eq "attached") {
            $created.Add($entry.Value)
        }
    }
} catch {
    for ($index = $created.Count - 1; $index -ge 0; $index--) {
        try {
            [System.IO.Directory]::Delete($created[$index])
        } catch {
            # Preserve the original connector error.
        }
    }
    throw
}

[ordered]@{ ok = $true; action = "attached"; skill = $canonical; links = $result } |
    ConvertTo-Json -Depth 5
