param([Parameter(Mandatory=$true)][string]$PlanFile,[switch]$Apply)
$ErrorActionPreference='Stop'
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
$workspace=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$plans=[IO.Path]::GetFullPath((Join-Path $workspace '.local/storage/plans'))
$planPath=[IO.Path]::GetFullPath($PlanFile)
if([IO.Path]::GetDirectoryName($planPath) -cne $plans -or [IO.Path]::GetFileName($planPath) -notmatch '^[a-f0-9-]{36}\.json$'){throw 'Plan outside owned plan directory'}
function Assert-NoLink([string]$value){
  $cursor=$value
  while($cursor){
    if(Test-Path -LiteralPath $cursor){$item=Get-Item -LiteralPath $cursor -Force;if($item.Attributes -band [IO.FileAttributes]::ReparsePoint){throw "Reparse point: $cursor"}}
    $parent=[IO.Path]::GetDirectoryName($cursor.TrimEnd('\'));if($parent -eq $cursor){break};$cursor=$parent
  }
}
Assert-NoLink $planPath
$plan=Get-Content -LiteralPath $planPath -Raw -Encoding UTF8 | ConvertFrom-Json
if([IO.Path]::GetFullPath($plan.root) -cne $workspace -or $plan.schemaVersion -ne 1){throw 'Plan workspace/schema differs'}
$now=([DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds())
if($now-[double]$plan.createdAt -gt 900000 -or $now -lt [double]$plan.createdAt){throw 'Stale or future cleanup plan'}
$audit=[ordered]@{schemaVersion=1;apply=[bool]$Apply;startedAt=[DateTime]::UtcNow.ToString('o');root=$workspace;deleted=@();skipped=@();planned=@();releasedBytes=[int64]0}
$auditFile=Join-Path $workspace ('.local/storage/cleanup-'+[IO.Path]::GetFileName($planPath))
function Save-Audit {$audit | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath $auditFile -Encoding UTF8}
function Tree([string]$value){
  Assert-NoLink $value
  $queue=[Collections.Generic.Stack[string]]::new();$queue.Push($value);$files=[Collections.Generic.List[object]]::new()
  while($queue.Count){foreach($entry in @(Get-ChildItem -LiteralPath $queue.Pop() -Force)){
    if($entry.Attributes -band [IO.FileAttributes]::ReparsePoint){throw "Nested reparse point: $($entry.FullName)"}
    if($entry.PSIsContainer){$queue.Push($entry.FullName)}else{$files.Add($entry)}
  }}
  return $files
}
function Assert-Pins([string]$relative){
  $pinsFile=Join-Path $workspace '.local/storage/pins.json';Assert-NoLink $pinsFile
  if(Test-Path -LiteralPath $pinsFile){foreach($pin in @(Get-Content -LiteralPath $pinsFile -Raw -Encoding UTF8 | ConvertFrom-Json)){
    if(-not $pin.relative -or -not $pin.reason){throw 'Invalid protection pin'}
    if($relative -eq $pin.relative -or $relative.StartsWith($pin.relative+'/',[StringComparison]::OrdinalIgnoreCase) -or $pin.relative.StartsWith($relative+'/',[StringComparison]::OrdinalIgnoreCase)){throw ('Pinned: '+$pin.reason)}
  }}
}
foreach($candidate in $plan.delete){
  try{
    $relative=[string]$candidate.relative
    if($relative.Contains('\') -or $relative.Split('/') -contains '..' -or $relative.Split('/') -contains '.'){throw 'Invalid relative path'}
    $absolute=[IO.Path]::GetFullPath((Join-Path $workspace $relative))
    if(-not $absolute.StartsWith($workspace+'\',[StringComparison]::Ordinal)){throw 'Outside workspace'}
    $incremental=$relative -cmatch '^src-tauri/(text-service/)?target/(debug|release)/incremental/[a-zA-Z0-9_]+-[a-z0-9]+$'
    $context=$relative -cmatch '^\.local/(data-acceptance|windows-update|installed-startup)/[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}$'
    if($candidate.kind -eq 'incremental' -and $incremental){
      $active=@(Get-CimInstance Win32_Process | Where-Object {$_.Name -match '^(cargo|rustc|rust-analyzer|clippy-driver|link|lld-link|sccache)\.exe$'})
      if($active.Count){throw 'Active compiler/linker; incremental cache protected'}
    }elseif($candidate.kind -eq 'context' -and $context){
      $id=[IO.Path]::GetFileName($absolute)
      if($candidate.id -cne $id){throw 'Context identity differs'}
      $manifestPath=Join-Path $workspace ('.local/storage/contexts/'+$id+'.json');Assert-NoLink $manifestPath
      $record=Get-Content -LiteralPath $manifestPath -Raw -Encoding UTF8 | ConvertFrom-Json
      if($record.owner -cne 'sayweave-validation-v1' -or $record.relative -cne $relative -or $record.status -eq 'active' -or -not $record.cleanupConfirmed -or $record.pin){throw 'Context is active, unowned, pinned or cleanup unconfirmed'}
      $processes=@(Get-CimInstance Win32_Process | Where-Object {($_.ExecutablePath -and $_.ExecutablePath.StartsWith($absolute+'\',[StringComparison]::OrdinalIgnoreCase)) -or ($_.CommandLine -and $_.CommandLine.Contains($id))})
      if($processes.Count){throw 'Context still referenced by a process'}
      if($record.status -ne 'succeeded'){
        $records=@(Get-ChildItem -LiteralPath (Join-Path $workspace '.local/storage/contexts') -Filter '*.json' | ForEach-Object {Get-Content -LiteralPath $_.FullName -Raw -Encoding UTF8 | ConvertFrom-Json} | Where-Object {$_.status -in @('failed','cancelled','interrupted')} | Sort-Object finishedAt -Descending)
        $rank=0;while($rank -lt $records.Count -and $records[$rank].id -cne $id){$rank++}
        if($rank -lt 3 -and $now-[double]$record.finishedAt -le 604800000){throw 'Failed context remains in retention window'}
      }
    }else{throw 'Outside cleanup allowlist'}
    Assert-Pins $relative;Assert-NoLink $absolute
    if(-not (Test-Path -LiteralPath $absolute)){throw 'Already absent; no deletion credited'}
    $files=@(Tree $absolute);$bytes=[int64](($files | Measure-Object Length -Sum).Sum)
    if($bytes -ne [int64]$candidate.bytes -or $files.Count -ne [int64]$candidate.files){throw 'Snapshot changed since plan'}
    $deadline=[DateTimeOffset]::FromUnixTimeMilliseconds([int64][double]$plan.createdAt).UtcDateTime.AddMilliseconds(1)
    if(@($files | Where-Object {$_.LastWriteTimeUtc -gt $deadline}).Count){throw 'Files changed since plan'}
    foreach($file in $files){$handle=[IO.File]::Open($file.FullName,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::None);$handle.Dispose()}
    Assert-NoLink $absolute;Assert-Pins $relative
    if($incremental -and @(Get-CimInstance Win32_Process | Where-Object {$_.Name -match '^(cargo|rustc|rust-analyzer|clippy-driver|link|lld-link|sccache)\.exe$'}).Count){throw 'Compiler started during verification; cache protected'}
    if($Apply){Remove-Item -LiteralPath $absolute -Recurse -Force;if(Test-Path -LiteralPath $absolute){throw 'Deletion incomplete'};$audit.deleted+=@{relative=$relative;bytes=$bytes};$audit.releasedBytes+=$bytes}
    else{$audit.planned+=@{relative=$relative;bytes=$bytes}}
  }catch{$audit.skipped+=@{relative=$candidate.relative;reason=$_.Exception.Message}}
  Save-Audit
}
$audit.finishedAt=[DateTime]::UtcNow.ToString('o');Save-Audit
Write-Output ($audit | ConvertTo-Json -Depth 10 -Compress)
