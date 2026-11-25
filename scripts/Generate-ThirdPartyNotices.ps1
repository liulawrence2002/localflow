<#
.SYNOPSIS
  Regenerates the dependency lists in THIRD_PARTY_NOTICES.md.

.DESCRIPTION
  Lists every Rust crate compiled into LocalFlow for Windows x64 (from `cargo metadata` and
  src-tauri\Cargo.lock) and every production npm package (from package-lock.json), each
  with its version and licence, grouped by licence.

  Only the part of THIRD_PARTY_NOTICES.md between the "BEGIN GENERATED" and "END GENERATED"
  markers is replaced. The hand-written sections above it (code ported from other projects,
  the bundled whisper.cpp programs and model, model credits) are kept as they are.

  Nothing is downloaded beyond what `cargo metadata` itself fetches when crates are missing
  from the local cache. Run it after changing dependencies, then run prettier on the file.

.PARAMETER Check
  Do not write; exit with code 1 if the file is out of date. The release workflow uses this.

.EXAMPLE
  .\scripts\Generate-ThirdPartyNotices.ps1
  .\scripts\Generate-ThirdPartyNotices.ps1 -Check
#>
param(
  [switch]$Check,
  [string]$Target = "x86_64-pc-windows-msvc"
)

$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$noticesPath = Join-Path $repoRoot "THIRD_PARTY_NOTICES.md"
$manifestPath = Join-Path $repoRoot "src-tauri\Cargo.toml"
$lockPath = Join-Path $repoRoot "package-lock.json"
$beginMarker = "<!-- BEGIN GENERATED: scripts/Generate-ThirdPartyNotices.ps1 -->"
$endMarker = "<!-- END GENERATED -->"

$cargoBin = Join-Path $env:USERPROFILE ".cargo\bin"
if ((Test-Path -LiteralPath $cargoBin) -and (($env:Path -split ";") -notcontains $cargoBin)) {
  $env:Path = "$cargoBin;$env:Path"
}
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
  throw "cargo was not found. Install Rust (scripts\Install-Prereqs.ps1 -Install)."
}

function Format-Repository([string]$Url) {
  if ([string]::IsNullOrWhiteSpace($Url)) { return "" }
  $clean = $Url.Trim() -replace "^git\+", "" -replace "\.git$", ""
  if ($clean -match "^git@github\.com:(.+)$") { $clean = "https://github.com/$($Matches[1])" }
  if ($clean -notmatch "^https?://") { return "" }
  return " - <$clean>"
}

# "MIT/Apache-2.0", "Apache-2.0 OR MIT" and "MIT OR Apache-2.0" mean the same choice; list
# them under one heading. Expressions with AND or parentheses are kept as written.
function Format-License([string]$Expression) {
  $text = $Expression.Trim()
  if ($text -match "\bAND\b|[()]" -or $text -eq "") { return $text }
  $choices = @($text -split "\s*/\s*|\s+OR\s+" | ForEach-Object { $_.Trim() } | Where-Object { $_ } | Sort-Object -Unique)
  return ($choices -join " OR ")
}

function Get-RustCrates {
  $json = & cargo metadata --format-version 1 --locked --manifest-path $manifestPath --filter-platform $Target
  if ($LASTEXITCODE -ne 0) { throw "cargo metadata failed (exit code $LASTEXITCODE)." }
  $metadata = ($json -join "`n") | ConvertFrom-Json

  $packages = @{}
  foreach ($package in $metadata.packages) { $packages[$package.id] = $package }
  $nodes = @{}
  foreach ($node in $metadata.resolve.nodes) { $nodes[$node.id] = $node }
  $workspace = @{}
  foreach ($id in $metadata.workspace_members) { $workspace[$id] = $true }

  # Everything reachable from the app through normal and build dependencies. Development
  # dependencies (tests, examples, benchmarks) are not part of what is shipped.
  $seen = @{}
  $queue = New-Object System.Collections.Generic.Queue[string]
  $queue.Enqueue($metadata.resolve.root)
  while ($queue.Count -gt 0) {
    $id = $queue.Dequeue()
    if ($seen.ContainsKey($id)) { continue }
    $seen[$id] = $true
    foreach ($dep in $nodes[$id].deps) {
      $shipped = @($dep.dep_kinds | Where-Object { $_.kind -ne "dev" }).Count -gt 0
      if ($shipped -and -not $seen.ContainsKey($dep.pkg)) { $queue.Enqueue($dep.pkg) }
    }
  }

  foreach ($id in $seen.Keys) {
    if ($workspace.ContainsKey($id)) { continue }
    $package = $packages[$id]
    $license = if ($package.license) { Format-License $package.license } elseif ($package.license_file) { "See the crate's licence file" } else { "Not stated" }
    [pscustomobject]@{
      Name       = $package.name
      Version    = $package.version
      License    = $license
      Repository = $package.repository
    }
  }
}

function Get-NpmPackages {
  # The root package's key is the empty string, which Windows PowerShell's ConvertFrom-Json
  # rejects. An empty key is the only place `""` can be followed by a colon.
  $text = (Get-Content -Raw -LiteralPath $lockPath) -replace '""\s*:', '"(root)":'
  $lock = $text | ConvertFrom-Json
  foreach ($property in $lock.packages.PSObject.Properties) {
    $key = $property.Name
    $entry = $property.Value
    # Only installed packages: not the root, not the local workspace packages, and not their
    # links. Development-only packages are marked "dev"; they never reach the app.
    if ($key -notmatch "(^|/)node_modules/") { continue }
    if ($entry.link -or $entry.dev) { continue }
    $name = $key -replace "^.*node_modules/", ""
    $repository = ""
    if ($entry.PSObject.Properties.Name -contains "resolved" -and $entry.resolved) {
      $repository = "https://www.npmjs.com/package/$name"
    }
    [pscustomobject]@{
      Name       = $name
      Version    = $entry.version
      License    = if ($entry.license) { Format-License "$($entry.license)" } else { "Not stated" }
      Repository = $repository
    }
  }
}

function Format-Groups($Items, [string]$Kind) {
  $lines = New-Object System.Collections.Generic.List[string]
  $groups = $Items | Group-Object License | Sort-Object @{ Expression = { $_.Count }; Descending = $true }, Name
  foreach ($group in $groups) {
    $count = $group.Count
    $noun = if ($count -eq 1) { $Kind } else { "${Kind}s" }
    $lines.Add("### $($group.Name) ($count $noun)")
    $lines.Add("")
    foreach ($item in ($group.Group | Sort-Object Name, Version)) {
      $lines.Add("- ``$($item.Name)`` $($item.Version)$(Format-Repository $item.Repository)")
    }
    $lines.Add("")
  }
  return $lines
}

$crates = @(Get-RustCrates | Sort-Object Name, Version -Unique)
$npm = @(Get-NpmPackages | Sort-Object Name, Version -Unique)

$generated = New-Object System.Collections.Generic.List[string]
$generated.Add($beginMarker)
$generated.Add("")
$generated.Add("## Rust crates")
$generated.Add("")
$generated.Add("$($crates.Count) crates are compiled into ``localflow.exe`` or its build for Windows x64, as locked in ``src-tauri/Cargo.lock`` (normal and build dependencies; test-only crates are left out). Where a crate offers a choice of licences, it is used under any of them. Each crate's own licence file, with its copyright notice, is in its source package on crates.io.")
$generated.Add("")
foreach ($line in (Format-Groups $crates "crate")) { $generated.Add($line) }
$generated.Add("## npm packages")
$generated.Add("")
$generated.Add("$($npm.Count) production npm packages are bundled into the settings window, as locked in ``package-lock.json``. Development tools (the compiler, test runner, linters and the Tauri CLI) are not shipped.")
$generated.Add("")
foreach ($line in (Format-Groups $npm "package")) { $generated.Add($line) }
$generated.Add($endMarker)

$current = Get-Content -Raw -LiteralPath $noticesPath
$start = $current.IndexOf($beginMarker)
$finish = $current.IndexOf($endMarker)
if ($start -lt 0 -or $finish -lt $start) {
  throw "THIRD_PARTY_NOTICES.md has no generated section. Restore the '$beginMarker' and '$endMarker' lines."
}
$before = $current.Substring(0, $start)
$after = $current.Substring($finish + $endMarker.Length)
$updated = $before + ($generated -join "`n") + $after
$updated = $updated -replace "`r`n", "`n"

if ($Check) {
  if (($current -replace "`r`n", "`n") -ne $updated) {
    Write-Host "THIRD_PARTY_NOTICES.md is out of date. Run scripts\Generate-ThirdPartyNotices.ps1."
    exit 1
  }
  Write-Host "THIRD_PARTY_NOTICES.md is up to date ($($crates.Count) crates, $($npm.Count) npm packages)."
  exit 0
}

[System.IO.File]::WriteAllText($noticesPath, $updated, (New-Object System.Text.UTF8Encoding($false)))
Write-Host "Updated THIRD_PARTY_NOTICES.md: $($crates.Count) crates, $($npm.Count) npm packages."
