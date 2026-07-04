$ErrorActionPreference = "Stop"

$cargoBin = Join-Path $env:USERPROFILE ".cargo\bin"
if ((Test-Path -LiteralPath $cargoBin) -and (($env:Path -split ";") -notcontains $cargoBin)) {
  $env:Path = "$cargoBin;$env:Path"
}

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
  throw "Rust/Cargo is required before building the installer."
}

npm install
npm run tauri:build
