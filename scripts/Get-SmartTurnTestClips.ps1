<#
.SYNOPSIS
  Downloads a labelled sample of the Smart Turn v3.2 test set for the turn-judge benchmark.

.DESCRIPTION
  Benchmark data only, downloaded when you run this. LocalFlow itself never downloads data
  and never contacts the network while it runs. Used by src-tauri/examples/turn_judge_bench.rs.

  Source: the Hugging Face dataset pipecat-ai/smart-turn-data-v3.2-test (Pipecat, CC-BY-4.0),
  read through the public datasets-server "filter" API, which returns one signed FLAC link per
  row. Only English rows are taken. Within each group (human or synthetic speech) and label
  (complete or incomplete turn) the rows are sorted by their id and the first N are kept, so
  the sample is the same on every run and unrelated to the order of the dataset.

  Writes <OutDir>\manifest.csv and one r<row>.flac per clip (16 kHz mono, about 60 KB each).
  The clips are third-party data: keep them out of the repository and delete the folder when
  the benchmark is done.

.EXAMPLE
  .\scripts\Get-SmartTurnTestClips.ps1 -OutDir $env:TEMP\smart-turn-data
#>
param(
  [Parameter(Mandatory = $true)]
  [string]$OutDir,
  [int]$HumanPerLabel = 800,
  [int]$SyntheticPerLabel = 200,
  [int]$Parallel = 6
)

$ErrorActionPreference = "Stop"

$dataset = "pipecat-ai/smart-turn-data-v3.2-test"
$api = "https://datasets-server.huggingface.co/filter"
$assetPrefix = "https://datasets-server.huggingface.co/cached-assets/$dataset/"
$pageSize = 100
$minimumFreeBytes = 1.5GB

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$OutDir = (Resolve-Path -LiteralPath $OutDir).Path
if (Get-ChildItem -LiteralPath $OutDir -Force | Select-Object -First 1) {
  throw "$OutDir is not empty. Give a new, empty folder."
}

function Assert-FreeSpace {
  $drive = Get-PSDrive -Name ((Split-Path -Qualifier $OutDir).TrimEnd(':'))
  if ($drive.Free -lt $minimumFreeBytes) {
    throw ("Only {0:N1} GB free on {1}: stopping." -f ($drive.Free / 1GB), $drive.Name)
  }
}

function Get-FilteredRows {
  param([string]$Where)
  $rows = New-Object System.Collections.Generic.List[object]
  $offset = 0
  while ($true) {
    $query = "dataset=" + [uri]::EscapeDataString($dataset) + "&config=default&split=train" +
      "&where=" + [uri]::EscapeDataString($Where) + "&offset=$offset&length=$pageSize"
    $page = $null
    for ($attempt = 1; $attempt -le 4; $attempt++) {
      try {
        $page = Invoke-RestMethod -Uri "$api`?$query" -TimeoutSec 90
        break
      } catch {
        if ($attempt -eq 4) { throw }
        Start-Sleep -Seconds (5 * $attempt)
      }
    }
    foreach ($entry in $page.rows) { $rows.Add($entry) }
    $offset += $pageSize
    if ($page.rows.Count -eq 0 -or $offset -ge [int]$page.num_rows_total) { break }
  }
  return , $rows
}

function Format-Flag {
  param($Value)
  if ($null -eq $Value) { return "" }
  if ($Value) { return "1" }
  return "0"
}

$groups = @(
  @{ Name = "human"; Where = "`"language`"='eng' AND `"synthetic`"=false"; PerLabel = $HumanPerLabel },
  @{ Name = "synthetic"; Where = "`"language`"='eng' AND `"synthetic`"=true"; PerLabel = $SyntheticPerLabel }
)

$selected = New-Object System.Collections.Generic.List[object]
foreach ($group in $groups) {
  if ($group.PerLabel -le 0) { continue }
  Write-Host "Listing $($group.Name) English rows ..."
  $rows = Get-FilteredRows -Where $group.Where
  foreach ($label in @($true, $false)) {
    $pick = @($rows | Where-Object { $_.row.endpoint_bool -eq $label } |
        Sort-Object { [string]$_.row.id } | Select-Object -First $group.PerLabel)
    Write-Host ("  {0}, complete={1}: {2} of {3} rows" -f $group.Name, $label, $pick.Count,
      @($rows | Where-Object { $_.row.endpoint_bool -eq $label }).Count)
    foreach ($entry in $pick) { $selected.Add($entry) }
  }
}

# Everything below comes from the network: check it before it names a file or a link.
$guid = '^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$'
$manifest = New-Object System.Collections.Generic.List[string]
$manifest.Add("file,row_idx,id,endpoint,midfiller,endfiller,synthetic,source")
$curlConfig = New-Object System.Collections.Generic.List[string]
foreach ($entry in $selected) {
  $rowIdx = [long]$entry.row_idx
  $row = $entry.row
  $id = [string]$row.id
  $src = [string]$row.audio[0].src
  if ($id -notmatch $guid) { throw "Row $rowIdx has an unexpected id." }
  if (-not $src.StartsWith($assetPrefix) -or $src -match '["\s\\]') {
    throw "Row $rowIdx has an unexpected audio link."
  }
  $source = [string]$row.dataset
  if ($source -notmatch '^[A-Za-z0-9_-]{1,40}$') { $source = "other" }
  $file = "r$rowIdx.flac"
  $manifest.Add(("{0},{1},{2},{3},{4},{5},{6},{7}" -f $file, $rowIdx, $id,
      (Format-Flag $row.endpoint_bool), (Format-Flag $row.midfiller),
      (Format-Flag $row.endfiller), (Format-Flag $row.synthetic), $source))
  $target = (Join-Path $OutDir $file) -replace '\\', '/'
  $curlConfig.Add("url = `"$src`"")
  $curlConfig.Add("output = `"$target`"")
}

Assert-FreeSpace
$configPath = Join-Path ([System.IO.Path]::GetTempPath()) ("localflow-clips-{0}.curl" -f $PID)
Set-Content -LiteralPath $configPath -Value $curlConfig -Encoding Ascii
try {
  Write-Host "Downloading $($selected.Count) clips ..."
  # curl.exe ships with Windows 10 and later; no telemetry, no user data.
  & curl.exe --parallel --parallel-max $Parallel --fail --silent --show-error --retry 3 `
    --retry-delay 2 --max-time 120 --config $configPath
} finally {
  Remove-Item -LiteralPath $configPath -ErrorAction SilentlyContinue
}

$missing = @($manifest | Select-Object -Skip 1 | ForEach-Object { $_.Split(',')[0] } |
    Where-Object { -not (Test-Path -LiteralPath (Join-Path $OutDir $_)) })
Set-Content -LiteralPath (Join-Path $OutDir "manifest.csv") -Value $manifest -Encoding Ascii
Assert-FreeSpace

$bytes = (Get-ChildItem -LiteralPath $OutDir -Filter *.flac | Measure-Object Length -Sum).Sum
Write-Host ("Saved {0} clips, {1:N1} MB, in {2}" -f ($selected.Count - $missing.Count),
  ($bytes / 1MB), $OutDir)
if ($missing.Count -gt 0) {
  Write-Warning "$($missing.Count) clips failed to download; the benchmark skips them."
}
