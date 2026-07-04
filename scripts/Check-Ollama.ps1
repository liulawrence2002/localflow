$ErrorActionPreference = "Stop"

if (-not (Get-Command ollama -ErrorAction SilentlyContinue)) {
  throw "Ollama is not installed or is not on PATH."
}

ollama list

try {
  Invoke-RestMethod -Uri "http://127.0.0.1:11434/api/tags" -Method Get -TimeoutSec 3 | Out-Null
  Write-Host "Ollama localhost API is available."
} catch {
  Write-Warning "Ollama CLI exists, but the localhost API did not respond."
}
