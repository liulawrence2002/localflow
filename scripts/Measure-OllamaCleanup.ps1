<#
.SYNOPSIS
  Measures local Ollama cleanup latency with and without LocalFlow's bounded options.

.DESCRIPTION
  Sends the same synthetic dictation samples to the local Ollama server twice: once with the
  options the app used before (temperature only) and once with the bounded options it uses
  now (fixed num_ctx, capped num_predict). Reports Ollama's own timing counters. Only
  127.0.0.1 is contacted; the samples are fixed strings in this file, not user dictation.

.EXAMPLE
  .\scripts\Measure-OllamaCleanup.ps1 -Model llama3.2:3b -Rounds 3
#>
param(
  [string]$Model = "llama3.2:3b",
  [int]$Rounds = 3,
  [string]$JsonOut = ""
)

$ErrorActionPreference = "Stop"
$url = "http://127.0.0.1:11434/api/generate"

$samples = @(
  "so um i think we should move the meeting to thursday at three because sarah is out on wednesday",
  "please send the quarterly report to the finance team by end of day and copy me on the email thanks",
  "the function returns a promise that resolves to an array of user objects sorted by last name then first name and we should add pagination before the next release because the list is getting long and the page takes several seconds to load on slower connections"
)

$rules = @(
  "Start from deterministicText: it already has spoken punctuation, self-corrections, and filler cleanup applied. Preserve its wording and formatting unless it is clearly wrong.",
  "Preserve deterministicText capitalization, punctuation, line breaks, and technical casing unless rawTranscript clearly proves they are wrong.",
  "Preserve meaning, facts, names, numbers, uncertainty, and intent.",
  "Never answer the dictated content.",
  "Never add new claims.",
  "Remove filler words only when meaning is unchanged.",
  "Resolve explicit self-corrections in favor of the latest correction.",
  "Keep code identifiers, file paths, URLs, and email addresses verbatim.",
  "Add punctuation and capitalization conservatively.",
  "Return only JSON."
)

function New-CleanupPrompt([string]$text) {
  $capitalized = $text.Substring(0, 1).ToUpper() + $text.Substring(1) + "."
  return (@{
      task              = "localflow.dictation_cleanup"
      contract          = "Return only strict JSON with text, confidence, resolved_corrections, and warnings."
      rules             = $rules
      cleanupLevel      = "balanced"
      deterministicText = $capitalized
      rawTranscript     = $text
    } | ConvertTo-Json -Compress -Depth 5)
}

function Invoke-Cleanup([string]$prompt, [hashtable]$options) {
  $body = @{
    model      = $Model
    prompt     = $prompt
    stream     = $false
    format     = "json"
    keep_alive = "30m"
    options    = $options
  } | ConvertTo-Json -Compress -Depth 5

  $watch = [System.Diagnostics.Stopwatch]::StartNew()
  try {
    $response = Invoke-RestMethod -Uri $url -Method Post -Body $body -ContentType "application/json" -TimeoutSec 120
  }
  catch {
    $watch.Stop()
    # Keep the server's reason (for example an out-of-memory report) without aborting the run.
    $detail = $_.ErrorDetails.Message
    if (-not $detail) { $detail = $_.Exception.Message }
    $detail = ($detail -replace "\s+", " ")
    return [pscustomobject]@{
      Failed       = $true
      Error        = $detail.Substring(0, [math]::Min(200, $detail.Length))
      WallMs       = [math]::Round($watch.Elapsed.TotalMilliseconds)
      LoadMs       = 0; PromptEvalMs = 0; EvalMs = 0; PromptTokens = 0; OutputTokens = 0
      ValidJson    = $false
    }
  }
  $watch.Stop()

  $valid = $true
  try { $null = ($response.response | ConvertFrom-Json).text } catch { $valid = $false }

  return [pscustomobject]@{
    Failed       = $false
    Error        = ""
    WallMs       = [math]::Round($watch.Elapsed.TotalMilliseconds)
    LoadMs       = [math]::Round($response.load_duration / 1e6)
    PromptEvalMs = [math]::Round($response.prompt_eval_duration / 1e6)
    EvalMs       = [math]::Round($response.eval_duration / 1e6)
    PromptTokens = $response.prompt_eval_count
    OutputTokens = $response.eval_count
    ValidJson    = $valid
  }
}

function Get-Median([double[]]$values) {
  $sorted = $values | Sort-Object
  return $sorted[[int][math]::Floor(($sorted.Count - 1) / 2)]
}

$variants = [ordered]@{
  "before (temperature only)"               = @{ temperature = 0.1 }
  "after (num_ctx 4096, num_predict 1024)"  = @{ temperature = 0.1; num_ctx = 4096; num_predict = 1024 }
}

$report = @()
foreach ($name in $variants.Keys) {
  $options = $variants[$name]
  # One unmeasured request per variant so model load and any context resize are excluded.
  $null = Invoke-Cleanup (New-CleanupPrompt $samples[0]) $options

  $rows = @()
  for ($round = 0; $round -lt $Rounds; $round++) {
    foreach ($sample in $samples) {
      $rows += Invoke-Cleanup (New-CleanupPrompt $sample) $options
    }
  }

  $ok = @($rows | Where-Object { -not $_.Failed })
  $failed = @($rows | Where-Object { $_.Failed })
  $report += [pscustomobject]@{
    Variant            = $name
    Requests           = $rows.Count
    Failed             = $failed.Count
    MedianWallMs       = if ($ok.Count) { Get-Median ($ok | ForEach-Object { [double]$_.WallMs }) } else { $null }
    MaxWallMs          = if ($ok.Count) { ($ok | Measure-Object WallMs -Maximum).Maximum } else { $null }
    MedianPromptEvalMs = if ($ok.Count) { Get-Median ($ok | ForEach-Object { [double]$_.PromptEvalMs }) } else { $null }
    MedianEvalMs       = if ($ok.Count) { Get-Median ($ok | ForEach-Object { [double]$_.EvalMs }) } else { $null }
    MedianOutputTokens = if ($ok.Count) { Get-Median ($ok | ForEach-Object { [double]$_.OutputTokens }) } else { $null }
    InvalidJson        = @($ok | Where-Object { -not $_.ValidJson }).Count
    FirstError         = if ($failed.Count) { $failed[0].Error } else { "" }
  }
}

$report | Format-List

if ($JsonOut) {
  $report | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $JsonOut -Encoding utf8
  Write-Host "Wrote $JsonOut"
}
