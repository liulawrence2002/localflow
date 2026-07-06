#!/usr/bin/env bash
set -euo pipefail

if ! command -v apt-get >/dev/null 2>&1; then
  echo "This installer currently supports Ubuntu/Debian systems with apt-get." >&2
  exit 1
fi

sudo apt-get update
sudo apt-get install -y \
  build-essential \
  curl \
  file \
  libasound2-dev \
  libayatana-appindicator3-dev \
  libdbus-1-dev \
  libgtk-3-dev \
  libjavascriptcoregtk-4.1-dev \
  librsvg2-dev \
  libsoup-3.0-dev \
  libwebkit2gtk-4.1-dev \
  pkg-config \
  xdotool \
  wtype

if ! command -v ollama >/dev/null 2>&1; then
  curl -fsSL https://ollama.com/install.sh | sh
fi

if ! ollama list >/dev/null 2>&1; then
  ollama serve >/tmp/localflow-ollama.log 2>&1 &
  sleep 2
fi
ollama pull llama3.2:3b

echo "Linux prerequisites are installed."
