$ErrorActionPreference = "Stop"

if (-not (Get-Command ollama -ErrorAction SilentlyContinue)) {
  throw "Ollama is not installed or is not on PATH."
}

# Default native cleanup model. Override with the LOCALFLOW_OLLAMA_MODEL env var to check
# whichever model you have selected in Settings > Models.
$requiredModel = if ($env:LOCALFLOW_OLLAMA_MODEL) { $env:LOCALFLOW_OLLAMA_MODEL } else { "llama3.2:3b" }
$models = ollama list
$models

try {
  $tags = Invoke-RestMethod -Uri "http://127.0.0.1:11434/api/tags" -Method Get -TimeoutSec 3
  Write-Host "Ollama localhost API is available."
  $hasRequiredModel = $false
  foreach ($model in $tags.models) {
    if ($model.model -eq $requiredModel -or $model.name -eq $requiredModel -or $model.model -eq "${requiredModel}:latest" -or $model.name -eq "${requiredModel}:latest") {
      $hasRequiredModel = $true
    }
  }

  if ($hasRequiredModel) {
    Write-Host "Required native cleanup model '$requiredModel' is available."
  } else {
    Write-Warning "Required native cleanup model '$requiredModel' was not found. Native dictation will fall back to raw Whisper text if cleanup fails."
  }
} catch {
  Write-Warning "Ollama CLI exists, but the localhost API did not respond."
}
