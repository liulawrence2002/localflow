$ErrorActionPreference = "Stop"

$cargoBin = Join-Path $env:USERPROFILE ".cargo\bin"
if ((Test-Path -LiteralPath $cargoBin) -and (($env:Path -split ";") -notcontains $cargoBin)) {
  $env:Path = "$cargoBin;$env:Path"
}

if (-not (Test-Path "node_modules")) {
  npm install
}

npm run tauri:dev
