#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

export PATH="$HOME/.local/bin:$HOME/.cargo/bin:$PATH"

if ! command -v node >/dev/null 2>&1 || ! command -v npm >/dev/null 2>&1; then
  echo "Node.js/npm are not on PATH. Install Node 22 or add ~/.local/bin to PATH." >&2
  exit 1
fi

if ! command -v cargo >/dev/null 2>&1; then
  echo "Rust/Cargo are not on PATH. Install Rust or add ~/.cargo/bin to PATH." >&2
  exit 1
fi

if ! command -v ollama >/dev/null 2>&1; then
  echo "Ollama is not installed. Run scripts/Install-Prereqs-linux.sh first." >&2
  exit 1
fi

if ! curl -fsS http://127.0.0.1:11434/api/tags >/dev/null 2>&1; then
  ollama serve >/tmp/localflow-ollama.log 2>&1 &
  sleep 2
fi

if ! ollama list | awk '{print $1}' | grep -Fxq "llama3.2:3b"; then
  ollama pull llama3.2:3b
fi

if [ ! -d node_modules ]; then
  npm install
fi

npm run tauri:dev
