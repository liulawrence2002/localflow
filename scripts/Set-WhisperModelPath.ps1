param(
  [Parameter(Mandatory = $true)]
  [string]$ModelPath
)

$ErrorActionPreference = "Stop"

if (-not (Test-Path -LiteralPath $ModelPath)) {
  throw "Model path does not exist: $ModelPath"
}

$configDir = Join-Path $env:APPDATA "LocalFlow"
New-Item -ItemType Directory -Force -Path $configDir | Out-Null
$configPath = Join-Path $configDir "model-path.json"

@{
  whisperModelPath = (Resolve-Path -LiteralPath $ModelPath).Path
  updatedAt = (Get-Date).ToUniversalTime().ToString("o")
} | ConvertTo-Json | Set-Content -LiteralPath $configPath -Encoding UTF8

Write-Host "Configured Whisper model path at $configPath"
