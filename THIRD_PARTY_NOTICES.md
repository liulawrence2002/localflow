# Third-party notices

LocalFlow is licensed under MIT OR Apache-2.0 (see `LICENSE-MIT` and `LICENSE-APACHE`). It
includes, adapts or works with the third-party code and models below, each under its own
licence. This file is installed next to `localflow.exe`.

- [Code adapted into LocalFlow's source](#code-adapted-into-localflows-source)
- [Shipped in the installer](#shipped-in-the-installer)
- [Models you download with the scripts](#models-you-download-with-the-scripts)
- [Rust crates](#rust-crates) and [npm packages](#npm-packages), generated from the lock
  files by `scripts/Generate-ThirdPartyNotices.ps1`

## Code adapted into LocalFlow's source

### Talon community

`src-tauri/src/coding_agents/spoken_forms.rs` ports the spoken-form generation of
`core/create_spoken_forms.py`, and `src-tauri/src/coding_agents/phrases.rs` follows the idea
of its `PhraseReplacer`, from Talon community (<https://github.com/talonhub/community>).

```text
MIT License

Copyright (c) 2021 Jeff Knaus, Ryan Hileman, Zac Roberts, Tony Grosinger, Talon community contributors

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## Shipped in the installer

### sonora: WebRTC echo cancellation (AEC3)

Conversation mode's echo cancellation is the `sonora` crate, a pure-Rust port of the WebRTC
audio processing module (<https://github.com/dignifiedquire/sonora>), compiled into
`localflow.exe`.

```text
Copyright (c) 2011, The WebRTC Project Authors. All rights reserved.
Copyright (c) 2016, Arun Raghavan and contributors. All rights reserved.
Copyright (c) 2026, dignifiedquire. All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are
met:

  * Redistributions of source code must retain the above copyright
    notice, this list of conditions and the following disclaimer.

  * Redistributions in binary form must reproduce the above copyright
    notice, this list of conditions and the following disclaimer in
    the documentation and/or other materials provided with the
    distribution.

  * Neither the name of Google nor the names of its contributors may
    be used to endorse or promote products derived from this software
    without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
"AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT
HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT
LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY
THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
(INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```

### whisper.cpp and ggml

The installer ships `whisper-cli.exe`, `whisper-server.exe`, `whisper.dll` and the `ggml*.dll`
files from the whisper.cpp v1.9.5 Windows x64 release (build b5454,
<https://github.com/ggml-org/whisper.cpp>), unmodified. The in-process speech engine,
transcribe.cpp (<https://github.com/handy-computer/transcribe.cpp>, MIT, "Copyright (c) 2026
The transcribe.cpp authors"), is compiled into `localflow.exe` with its own copy of ggml,
Silero VAD support code and miniz, all MIT.

```text
MIT License

Copyright (c) 2023-2026 The ggml authors

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

The whisper.cpp programs also contain, from its `examples` folder:

- cpp-httplib, "Copyright (c) 2025 Yuji Hirose. All rights reserved.", MIT
  (<https://github.com/yhirose/cpp-httplib>), in `whisper-server.exe`;
- JSON for Modern C++, "Copyright 2013-2022 Niels Lohmann", MIT
  (<https://github.com/nlohmann/json>);
- miniaudio by David Reid, public domain or MIT No Attribution (<https://miniaud.io>);
- stb_vorbis by Sean Barrett, public domain or MIT (<https://github.com/nothings/stb>).

The MIT licence text for each is the one above with its own copyright line.

### Whisper tiny.en model

`ggml-tiny.en-q5_1.bin` is OpenAI's Whisper tiny.en model, converted to the ggml format and
quantised by the whisper.cpp project (<https://huggingface.co/ggerganov/whisper.cpp>, MIT).
The weights are OpenAI's (<https://github.com/openai/whisper>):

```text
MIT License

Copyright (c) 2022 OpenAI

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

### Microsoft components

- **Visual C++ runtime.** `msvcp140.dll`, `vcruntime140.dll`, `vcruntime140_1.dll` and
  `vcomp140.dll` are installed next to `localflow.exe` and the whisper.cpp programs. They are
  Microsoft Visual C++ Redistributable files, distributed under the Microsoft Visual Studio
  licence terms for redistributable code.
- **WebView2.** The installer embeds Microsoft's WebView2 bootstrapper, which installs the
  WebView2 Runtime from Microsoft only if it is missing (Windows 11 includes it).
- The installers are built with NSIS (zlib/libpng licence) and the WiX Toolset.

## Models you download with the scripts

None of these is in the installer. Each script downloads one pinned revision from the
source named here, checks every file's SHA-256, and puts it in `%LOCALAPPDATA%\LocalFlow`
(or a checkout's `.localflow-runtime`).

- **NVIDIA Parakeet TDT 0.6B v3** (`Get-AsrModel.ps1`), the default speech model.
  "parakeet-tdt-0.6b-v3" by NVIDIA (<https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3>),
  licensed under [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/). Changes: converted
  to GGUF for transcribe.cpp and quantised to Q5_K_M by handy-computer
  (<https://huggingface.co/handy-computer/parakeet-tdt-0.6b-v3-gguf>), which repackages the
  weights, renames and re-lays-out tensors and stores them at reduced precision; the weights
  were not retrained or fine-tuned. LocalFlow uses that file as published.
- **Gradium Phonon 31m (English)** (`Get-PhononModel.ps1`), the conversation voice. Weights and
  default voice by Gradium (<https://huggingface.co/gradium/phonon-31m-en>), Apache-2.0.
- **Silero VAD v6.2** (`Get-VadModel.ps1`), voice-activity detection. By the Silero Team
  (<https://github.com/snakers4/silero-vad>), MIT, "Copyright (c) 2020-present Silero Team";
  converted to GGUF by handy-computer
  (<https://huggingface.co/handy-computer/silero-vad-v6.2-gguf>), not retrained.
- **Pipecat Smart Turn v3.2** (`Get-SmartTurnModel.ps1`), the end-of-turn model. By Daily for
  the Pipecat project (<https://huggingface.co/pipecat-ai/smart-turn-v3>,
  <https://github.com/pipecat-ai/smart-turn>), BSD 2-Clause.
- **Fish Audio S2 Pro** (`Get-FishModel.ps1`), the optional read-aloud voice. By Fish Audio
  (<https://huggingface.co/fishaudio/s2-pro>) under the Fish Audio Research License: research
  and personal non-commercial use only; commercial use needs a licence from Fish Audio. GGUF
  conversion by rodrigomt (<https://huggingface.co/rodrigomt/s2-pro-gguf>). Never bundled; the
  script saves the licence next to the model.
- **s2.cpp** (`Build-FishS2.ps1`), the engine for Fish S2, built from source on your computer
  at a pinned commit (<https://github.com/rodrigomatta/s2.cpp>), under the licence in its
  repository. The script copies that licence next to the engine.
- **whisper.cpp programs** (`Get-WhisperRuntime.ps1`, for building from source): the same files
  the installer ships, described above.

The language model is whatever you install in Ollama (<https://ollama.com>); its licence is
the one its publisher gives, and LocalFlow does not ship or download it.

## Notes on the generated lists

- The crates under MPL-2.0 (the CSS parser and selector crates that Tauri uses, and
  `option-ext`) are used unmodified. Their source code is available from crates.io and from
  the repositories linked below.
- Crates and packages offering a choice of licences are listed under the choice, with the
  licences in alphabetical order.

<!-- BEGIN GENERATED: scripts/Generate-ThirdPartyNotices.ps1 -->

## Rust crates

447 crates are compiled into `localflow.exe` or its build for Windows x64, as locked in `src-tauri/Cargo.lock` (normal and build dependencies; test-only crates are left out). Where a crate offers a choice of licences, it is used under any of them. Each crate's own licence file, with its copyright notice, is in its source package on crates.io.

### Apache-2.0 OR MIT (277 crates)

- `ahash` 0.8.12 - <https://github.com/tkaitchuck/ahash>
- `allocator-api2` 0.2.21 - <https://github.com/zakarumych/allocator-api2>
- `anyhow` 1.0.103 - <https://github.com/dtolnay/anyhow>
- `arboard` 3.6.1 - <https://github.com/1Password/arboard>
- `atomic-waker` 1.1.2 - <https://github.com/smol-rs/atomic-waker>
- `autocfg` 1.5.1 - <https://github.com/cuviper/autocfg>
- `base64` 0.13.1 - <https://github.com/marshallpierce/rust-base64>
- `base64` 0.22.1 - <https://github.com/marshallpierce/rust-base64>
- `bitflags` 1.3.2 - <https://github.com/bitflags/bitflags>
- `bitflags` 2.13.0 - <https://github.com/bitflags/bitflags>
- `bit-set` 0.11.1 - <https://github.com/contain-rs/bit-vec>
- `bit-set` 0.8.0 - <https://github.com/contain-rs/bit-set>
- `bit-vec` 0.10.2 - <https://github.com/contain-rs/bit-vec>
- `bit-vec` 0.8.0 - <https://github.com/contain-rs/bit-vec>
- `block-buffer` 0.10.4 - <https://github.com/RustCrypto/utils>
- `borsh` 1.8.1 - <https://github.com/near/borsh-rs>
- `bs58` 0.5.1 - <https://github.com/Nullus157/bs58-rs>
- `camino` 1.2.4 - <https://github.com/camino-rs/camino>
- `cargo_toml` 0.22.3 - <https://gitlab.com/lib.rs/cargo_toml>
- `cargo-platform` 0.1.9 - <https://github.com/rust-lang/cargo>
- `cc` 1.2.65 - <https://github.com/rust-lang/cc-rs>
- `cfg-if` 1.0.4 - <https://github.com/rust-lang/cfg-if>
- `chacha20` 0.10.2 - <https://github.com/RustCrypto/stream-ciphers>
- `chrono` 0.4.45 - <https://github.com/chronotope/chrono>
- `cmake` 0.1.58 - <https://github.com/rust-lang/cmake-rs>
- `cookie` 0.18.1 - <https://github.com/SergioBenitez/cookie-rs>
- `cpufeatures` 0.2.17 - <https://github.com/RustCrypto/utils>
- `cpufeatures` 0.3.1 - <https://github.com/RustCrypto/utils>
- `crc32fast` 1.5.0 - <https://github.com/srijs/rust-crc32fast>
- `crossbeam-channel` 0.5.15 - <https://github.com/crossbeam-rs/crossbeam>
- `crossbeam-deque` 0.8.8 - <https://github.com/crossbeam-rs/crossbeam>
- `crossbeam-epoch` 0.9.21 - <https://github.com/crossbeam-rs/crossbeam>
- `crossbeam-utils` 0.8.21 - <https://github.com/crossbeam-rs/crossbeam>
- `crypto-common` 0.1.7 - <https://github.com/RustCrypto/traits>
- `ctor` 0.8.0 - <https://github.com/mmastrac/rust-ctor>
- `ctor-proc-macro` 0.0.7 - <https://github.com/mmastrac/rust-ctor>
- `daachorse` 3.0.3 - <https://github.com/daac-tools/daachorse>
- `dary_heap` 0.3.9 - <https://github.com/hanmertens/dary_heap>
- `dasp_frame` 0.11.0 - <https://github.com/rustaudio/dasp>
- `dasp_sample` 0.11.0 - <https://github.com/rustaudio/sample>
- `deranged` 0.5.8 - <https://github.com/jhpratt/deranged>
- `derive_builder` 0.20.2 - <https://github.com/colin-kiegel/rust-derive-builder>
- `derive_builder_core` 0.20.2 - <https://github.com/colin-kiegel/rust-derive-builder>
- `derive_builder_macro` 0.20.2 - <https://github.com/colin-kiegel/rust-derive-builder>
- `digest` 0.10.7 - <https://github.com/RustCrypto/traits>
- `dirs` 6.0.0 - <https://github.com/soc/dirs-rs>
- `dirs-sys` 0.5.0 - <https://github.com/dirs-dev/dirs-sys-rs>
- `displaydoc` 0.2.6 - <https://github.com/yaahc/displaydoc>
- `downcast-rs` 2.0.2 - <https://github.com/marcianx/downcast-rs>
- `dtoa` 1.0.11 - <https://github.com/dtolnay/dtoa>
- `dtor` 0.3.0 - <https://github.com/mmastrac/rust-ctor>
- `dtor-proc-macro` 0.0.6 - <https://github.com/mmastrac/rust-ctor>
- `dyn-clone` 1.0.20 - <https://github.com/dtolnay/dyn-clone>
- `dyn-hash` 1.0.0 - <https://github.com/dtolnay/dyn-hash>
- `either` 1.16.0 - <https://github.com/rayon-rs/either>
- `equivalent` 1.0.2 - <https://github.com/indexmap-rs/equivalent>
- `erased-serde` 0.4.10 - <https://github.com/dtolnay/erased-serde>
- `fallible-iterator` 0.3.0 - <https://github.com/sfackler/rust-fallible-iterator>
- `fallible-streaming-iterator` 0.1.9 - <https://github.com/sfackler/fallible-streaming-iterator>
- `fastrand` 2.4.1 - <https://github.com/smol-rs/fastrand>
- `fdeflate` 0.3.7 - <https://github.com/image-rs/fdeflate>
- `filetime` 0.2.30 - <https://github.com/alexcrichton/filetime>
- `find-msvc-tools` 0.1.9 - <https://github.com/rust-lang/cc-rs>
- `flate2` 1.1.9 - <https://github.com/rust-lang/flate2-rs>
- `fnv` 1.0.7 - <https://github.com/servo/rust-fnv>
- `form_urlencoded` 1.2.2 - <https://github.com/servo/rust-url>
- `futures-channel` 0.3.32 - <https://github.com/rust-lang/futures-rs>
- `futures-core` 0.3.32 - <https://github.com/rust-lang/futures-rs>
- `futures-io` 0.3.32 - <https://github.com/rust-lang/futures-rs>
- `futures-macro` 0.3.32 - <https://github.com/rust-lang/futures-rs>
- `futures-sink` 0.3.32 - <https://github.com/rust-lang/futures-rs>
- `futures-task` 0.3.32 - <https://github.com/rust-lang/futures-rs>
- `futures-util` 0.3.32 - <https://github.com/rust-lang/futures-rs>
- `getrandom` 0.3.4 - <https://github.com/rust-random/getrandom>
- `getrandom` 0.4.3 - <https://github.com/rust-random/getrandom>
- `glob` 0.3.3 - <https://github.com/rust-lang/glob>
- `global-hotkey` 0.8.0 - <https://github.com/tauri-apps/global-hotkey>
- `half` 2.7.1 - <https://github.com/VoidStarKat/half-rs>
- `hashbrown` 0.12.3 - <https://github.com/rust-lang/hashbrown>
- `hashbrown` 0.14.5 - <https://github.com/rust-lang/hashbrown>
- `hashbrown` 0.16.1 - <https://github.com/rust-lang/hashbrown>
- `hashbrown` 0.17.1 - <https://github.com/rust-lang/hashbrown>
- `hashlink` 0.9.1 - <https://github.com/kyren/hashlink>
- `heck` 0.5.0 - <https://github.com/withoutboats/heck>
- `hex` 0.4.3 - <https://github.com/KokaKiwi/rust-hex>
- `html5ever` 0.38.0 - <https://github.com/servo/html5ever>
- `http` 1.4.2 - <https://github.com/hyperium/http>
- `httparse` 1.10.1 - <https://github.com/seanmonstar/httparse>
- `ident_case` 1.0.1 - <https://github.com/TedDriggs/ident_case>
- `idna` 1.1.0 - <https://github.com/servo/rust-url/>
- `idna_adapter` 1.2.2 - <https://github.com/hsivonen/idna_adapter>
- `image` 0.25.10 - <https://github.com/image-rs/image>
- `indexmap` 1.9.3 - <https://github.com/bluss/indexmap>
- `indexmap` 2.14.0 - <https://github.com/indexmap-rs/indexmap>
- `inventory` 0.3.25 - <https://github.com/dtolnay/inventory>
- `ipnet` 2.12.0 - <https://github.com/krisprice/ipnet>
- `itertools` 0.14.0 - <https://github.com/rust-itertools/itertools>
- `itertools` 0.15.0 - <https://github.com/rust-itertools/itertools>
- `itoa` 1.0.18 - <https://github.com/dtolnay/itoa>
- `jobserver` 0.1.34 - <https://github.com/rust-lang/jobserver-rs>
- `json-patch` 3.0.1 - <https://github.com/idubrov/json-patch>
- `jsonptr` 0.6.3 - <https://github.com/chanced/jsonptr>
- `keyboard-types` 0.7.0 - <https://github.com/pyfisch/keyboard-types>
- `lazy_static` 1.5.0 - <https://github.com/rust-lang-nursery/lazy-static.rs>
- `libc` 0.2.186 - <https://github.com/rust-lang/libc>
- `lock_api` 0.4.14 - <https://github.com/Amanieu/parking_lot>
- `log` 0.4.33 - <https://github.com/rust-lang/log>
- `maplit` 1.0.2 - <https://github.com/bluss/maplit>
- `markup5ever` 0.38.0 - <https://github.com/servo/html5ever>
- `matrixmultiply` 0.3.11 - <https://github.com/bluss/matrixmultiply/>
- `memmap2` 0.9.12 - <https://github.com/RazrFalcon/memmap2-rs>
- `mime` 0.3.17 - <https://github.com/hyperium/mime>
- `minimal-lexical` 0.2.1 - <https://github.com/Alexhuszagh/minimal-lexical>
- `monostate` 0.1.18 - <https://github.com/dtolnay/monostate>
- `monostate-impl` 0.1.18 - <https://github.com/dtolnay/monostate>
- `muda` 0.19.3 - <https://github.com/tauri-apps/muda>
- `ndarray` 0.17.2 - <https://github.com/rust-ndarray/ndarray>
- `num_cpus` 1.17.0 - <https://github.com/seanmonstar/num_cpus>
- `num-bigint` 0.4.8 - <https://github.com/rust-num/num-bigint>
- `num-complex` 0.4.6 - <https://github.com/rust-num/num-complex>
- `num-conv` 0.2.2 - <https://github.com/jhpratt/num-conv>
- `num-integer` 0.1.47 - <https://github.com/rust-num/num-integer>
- `num-rational` 0.4.2 - <https://github.com/rust-num/num-rational>
- `num-traits` 0.2.19 - <https://github.com/rust-num/num-traits>
- `once_cell` 1.21.4 - <https://github.com/matklad/once_cell>
- `parking_lot` 0.12.5 - <https://github.com/Amanieu/parking_lot>
- `parking_lot_core` 0.9.12 - <https://github.com/Amanieu/parking_lot>
- `paste` 1.0.15 - <https://github.com/dtolnay/paste>
- `pastey` 0.2.3 - <https://github.com/as1100k/pastey>
- `percent-encoding` 2.3.2 - <https://github.com/servo/rust-url/>
- `pin-project-lite` 0.2.17 - <https://github.com/taiki-e/pin-project-lite>
- `pkg-config` 0.3.33 - <https://github.com/rust-lang/pkg-config-rs>
- `png` 0.17.16 - <https://github.com/image-rs/image-png>
- `png` 0.18.1 - <https://github.com/image-rs/image-png>
- `powerfmt` 0.2.0 - <https://github.com/jhpratt/powerfmt>
- `ppv-lite86` 0.2.21 - <https://github.com/cryptocorrosion/cryptocorrosion>
- `primal-check` 0.3.4 - <https://github.com/huonw/primal>
- `proc-macro2` 1.0.106 - <https://github.com/dtolnay/proc-macro2>
- `proc-macro-crate` 3.5.0 - <https://github.com/bkchr/proc-macro-crate>
- `ptts` 0.7.1 - <https://github.com/gradium-ai/xn-ptts>
- `quick-error` 2.0.1 - <http://github.com/tailhook/quick-error>
- `quote` 1.0.46 - <https://github.com/dtolnay/quote>
- `rand` 0.10.4 - <https://github.com/rust-random/rand>
- `rand` 0.9.5 - <https://github.com/rust-random/rand>
- `rand_chacha` 0.9.0 - <https://github.com/rust-random/rand>
- `rand_core` 0.10.1 - <https://github.com/rust-random/rand_core>
- `rand_core` 0.9.5 - <https://github.com/rust-random/rand>
- `rand_distr` 0.5.1 - <https://github.com/rust-random/rand_distr>
- `rand_distr` 0.6.0 - <https://github.com/rust-random/rand_distr>
- `rapidhash` 4.5.1 - <https://github.com/hoxxep/rapidhash>
- `rawpointer` 0.2.1 - <https://github.com/bluss/rawpointer/>
- `rayon` 1.12.0 - <https://github.com/rayon-rs/rayon>
- `rayon-cond` 0.4.0 - <https://github.com/cuviper/rayon-cond>
- `rayon-core` 1.13.0 - <https://github.com/rayon-rs/rayon>
- `ref-cast` 1.0.25 - <https://github.com/dtolnay/ref-cast>
- `ref-cast-impl` 1.0.25 - <https://github.com/dtolnay/ref-cast>
- `regex` 1.12.4 - <https://github.com/rust-lang/regex>
- `regex-automata` 0.4.14 - <https://github.com/rust-lang/regex>
- `regex-syntax` 0.8.11 - <https://github.com/rust-lang/regex>
- `reqwest` 0.12.28 - <https://github.com/seanmonstar/reqwest>
- `rustc_version` 0.4.1 - <https://github.com/djc/rustc-version-rs>
- `rustc-hash` 2.1.3 - <https://github.com/rust-lang/rustc-hash>
- `rustfft` 6.4.1 - <https://github.com/ejmahler/RustFFT>
- `rustversion` 1.0.22 - <https://github.com/dtolnay/rustversion>
- `scopeguard` 1.2.0 - <https://github.com/bluss/scopeguard>
- `semver` 1.0.28 - <https://github.com/dtolnay/semver>
- `seq-macro` 0.3.6 - <https://github.com/dtolnay/seq-macro>
- `serde` 1.0.228 - <https://github.com/serde-rs/serde>
- `serde_core` 1.0.228 - <https://github.com/serde-rs/serde>
- `serde_derive` 1.0.228 - <https://github.com/serde-rs/serde>
- `serde_derive_internals` 0.29.1 - <https://github.com/serde-rs/serde>
- `serde_json` 1.0.150 - <https://github.com/serde-rs/json>
- `serde_repr` 0.1.20 - <https://github.com/dtolnay/serde-repr>
- `serde_spanned` 1.1.1 - <https://github.com/toml-rs/toml>
- `serde_urlencoded` 0.7.1 - <https://github.com/nox/serde_urlencoded>
- `serde_with` 3.21.0 - <https://github.com/jonasbb/serde_with/>
- `serde_with_macros` 3.21.0 - <https://github.com/jonasbb/serde_with/>
- `serde-untagged` 0.1.9 - <https://github.com/dtolnay/serde-untagged>
- `serialize-to-javascript` 0.1.2 - <https://github.com/chippers/serialize-to-javascript>
- `serialize-to-javascript-impl` 0.1.2 - <https://github.com/chippers/serialize-to-javascript>
- `servo_arc` 0.4.3 - <https://github.com/servo/stylo>
- `sha2` 0.10.9 - <https://github.com/RustCrypto/hashes>
- `shlex` 2.0.1 - <https://github.com/comex/rust-shlex>
- `siphasher` 1.0.3 - <https://github.com/jedisct1/rust-siphash>
- `smallvec` 1.15.2 - <https://github.com/servo/rust-smallvec>
- `socket2` 0.6.4 - <https://github.com/rust-lang/socket2>
- `softbuffer` 0.4.8 - <https://github.com/rust-windowing/softbuffer>
- `stable_deref_trait` 1.2.1 - <https://github.com/storyyeller/stable_deref_trait>
- `static_assertions` 1.1.0 - <https://github.com/nvzqz/static-assertions-rs>
- `strength_reduce` 0.2.4 - <http://github.com/ejmahler/strength_reduce>
- `string_cache` 0.9.0 - <https://github.com/servo/string-cache>
- `string_cache_codegen` 0.6.1 - <https://github.com/servo/string-cache>
- `string-interner` 0.20.0 - <https://github.com/robbepop/string-interner>
- `syn` 2.0.118 - <https://github.com/dtolnay/syn>
- `syn` 3.0.10 - <https://github.com/dtolnay/syn>
- `tar` 0.4.46 - <https://github.com/composefs/tar-rs>
- `tauri` 2.11.5 - <https://github.com/tauri-apps/tauri>
- `tauri-build` 2.6.3 - <https://github.com/tauri-apps/tauri>
- `tauri-codegen` 2.6.3 - <https://github.com/tauri-apps/tauri>
- `tauri-macros` 2.6.3 - <https://github.com/tauri-apps/tauri>
- `tauri-plugin` 2.6.3 - <https://github.com/tauri-apps/tauri>
- `tauri-plugin-global-shortcut` 2.3.2 - <https://github.com/tauri-apps/plugins-workspace>
- `tauri-plugin-single-instance` 2.4.5 - <https://github.com/tauri-apps/plugins-workspace>
- `tauri-runtime` 2.11.3 - <https://github.com/tauri-apps/tauri>
- `tauri-runtime-wry` 2.11.4 - <https://github.com/tauri-apps/tauri>
- `tauri-utils` 2.9.3 - <https://github.com/tauri-apps/tauri>
- `tendril` 0.5.0 - <https://github.com/servo/html5ever>
- `thiserror` 1.0.69 - <https://github.com/dtolnay/thiserror>
- `thiserror` 2.0.18 - <https://github.com/dtolnay/thiserror>
- `thiserror-impl` 1.0.69 - <https://github.com/dtolnay/thiserror>
- `thiserror-impl` 2.0.18 - <https://github.com/dtolnay/thiserror>
- `thread_local` 1.1.9 - <https://github.com/Amanieu/thread_local-rs>
- `time` 0.3.53 - <https://github.com/time-rs/time>
- `time-core` 0.1.9 - <https://github.com/time-rs/time>
- `time-macros` 0.2.31 - <https://github.com/time-rs/time>
- `toml` 0.9.12+spec-1.1.0 - <https://github.com/toml-rs/toml>
- `toml` 1.1.2+spec-1.1.0 - <https://github.com/toml-rs/toml>
- `toml_datetime` 0.7.5+spec-1.1.0 - <https://github.com/toml-rs/toml>
- `toml_datetime` 1.1.1+spec-1.1.0 - <https://github.com/toml-rs/toml>
- `toml_edit` 0.25.12+spec-1.1.0 - <https://github.com/toml-rs/toml>
- `toml_parser` 1.1.2+spec-1.1.0 - <https://github.com/toml-rs/toml>
- `toml_writer` 1.1.1+spec-1.1.0 - <https://github.com/toml-rs/toml>
- `tract-core` 0.23.10 - <https://github.com/sonos/tract>
- `tract-data` 0.23.10 - <https://github.com/sonos/tract>
- `tract-extra` 0.23.10 - <https://github.com/sonos/tract>
- `tract-hir` 0.23.10 - <https://github.com/sonos/tract>
- `tract-linalg` 0.23.10 - <https://github.com/sonos/tract>
- `tract-nnef` 0.23.10 - <https://github.com/sonos/tract>
- `tract-onnx` 0.23.10 - <https://github.com/sonos/tract>
- `tract-onnx-opl` 0.23.10 - <https://github.com/sonos/tract>
- `transpose` 0.2.3 - <https://github.com/ejmahler/transpose>
- `tray-icon` 0.24.1 - <https://github.com/tauri-apps/tray-icon>
- `typeid` 1.0.3 - <https://github.com/dtolnay/typeid>
- `typenum` 1.20.1 - <https://github.com/paholg/typenum>
- `unic-char-property` 0.9.0 - <https://github.com/open-i18n/rust-unic/>
- `unic-char-range` 0.9.0 - <https://github.com/open-i18n/rust-unic/>
- `unic-common` 0.9.0 - <https://github.com/open-i18n/rust-unic/>
- `unicode_categories` 0.1.1 - <https://github.com/swgillespie/unicode-categories>
- `unicode-normalization-alignments` 0.1.12 - <https://github.com/n1t0/unicode-normalization>
- `unicode-segmentation` 1.13.3 - <https://github.com/unicode-rs/unicode-segmentation>
- `unicode-xid` 0.2.6 - <https://github.com/unicode-rs/unicode-xid>
- `unic-ucd-ident` 0.9.0 - <https://github.com/open-i18n/rust-unic/>
- `unic-ucd-version` 0.9.0 - <https://github.com/open-i18n/rust-unic/>
- `url` 2.5.8 - <https://github.com/servo/rust-url>
- `utf-8` 0.7.6 - <https://github.com/SimonSapin/rust-utf8>
- `utf8_iter` 1.0.4 - <https://github.com/hsivonen/utf8_iter>
- `uuid` 1.23.4 - <https://github.com/uuid-rs/uuid>
- `vcpkg` 0.2.15 - <https://github.com/mcgoo/vcpkg-rs>
- `version_check` 0.9.5 - <https://github.com/SergioBenitez/version_check>
- `web_atoms` 0.2.5 - <https://github.com/servo/html5ever>
- `weezl` 0.1.12 - <https://github.com/image-rs/weezl>
- `windows` 0.54.0 - <https://github.com/microsoft/windows-rs>
- `windows` 0.61.3 - <https://github.com/microsoft/windows-rs>
- `windows_x86_64_msvc` 0.52.6 - <https://github.com/microsoft/windows-rs>
- `windows_x86_64_msvc` 0.53.1 - <https://github.com/microsoft/windows-rs>
- `windows-collections` 0.2.0 - <https://github.com/microsoft/windows-rs>
- `windows-core` 0.54.0 - <https://github.com/microsoft/windows-rs>
- `windows-core` 0.61.2 - <https://github.com/microsoft/windows-rs>
- `windows-future` 0.2.1 - <https://github.com/microsoft/windows-rs>
- `windows-implement` 0.60.2 - <https://github.com/microsoft/windows-rs>
- `windows-interface` 0.59.3 - <https://github.com/microsoft/windows-rs>
- `windows-link` 0.1.3 - <https://github.com/microsoft/windows-rs>
- `windows-link` 0.2.1 - <https://github.com/microsoft/windows-rs>
- `windows-numerics` 0.2.0 - <https://github.com/microsoft/windows-rs>
- `windows-result` 0.1.2 - <https://github.com/microsoft/windows-rs>
- `windows-result` 0.3.4 - <https://github.com/microsoft/windows-rs>
- `windows-strings` 0.4.2 - <https://github.com/microsoft/windows-rs>
- `windows-sys` 0.59.0 - <https://github.com/microsoft/windows-rs>
- `windows-sys` 0.60.2 - <https://github.com/microsoft/windows-rs>
- `windows-sys` 0.61.2 - <https://github.com/microsoft/windows-rs>
- `windows-targets` 0.52.6 - <https://github.com/microsoft/windows-rs>
- `windows-targets` 0.53.5 - <https://github.com/microsoft/windows-rs>
- `windows-threading` 0.1.0 - <https://github.com/microsoft/windows-rs>
- `windows-version` 0.1.7 - <https://github.com/microsoft/windows-rs>
- `window-vibrancy` 0.6.0 - <https://github.com/tauri-apps/tauri-plugin-vibrancy>
- `wry` 0.55.1 - <https://github.com/tauri-apps/wry>
- `xn` 0.2.15 - <https://github.com/LaurentMazare/xn>

### MIT (94 crates)

- `bytes` 1.12.0 - <https://github.com/tokio-rs/bytes>
- `cargo_metadata` 0.19.2 - <https://github.com/oli-obk/cargo_metadata>
- `castaway` 0.2.4 - <https://github.com/sagebind/castaway>
- `cfb` 0.7.3 - <https://github.com/mdsteele/rust-cfb>
- `cfg_aliases` 0.2.2 - <https://github.com/katharostech/cfg_aliases>
- `compact_str` 0.9.1 - <https://github.com/ParkMyCar/compact_str>
- `darling` 0.20.11 - <https://github.com/TedDriggs/darling>
- `darling` 0.23.0 - <https://github.com/TedDriggs/darling>
- `darling_core` 0.20.11 - <https://github.com/TedDriggs/darling>
- `darling_core` 0.23.0 - <https://github.com/TedDriggs/darling>
- `darling_macro` 0.20.11 - <https://github.com/TedDriggs/darling>
- `darling_macro` 0.23.0 - <https://github.com/TedDriggs/darling>
- `derive_more` 2.1.1 - <https://github.com/JelteF/derive_more>
- `derive_more-impl` 2.1.1 - <https://github.com/JelteF/derive_more>
- `derive-new` 0.7.0 - <https://github.com/nrc/derive-new>
- `dom_query` 0.27.0 - <https://github.com/niklak/dom_query>
- `dyn-stack` 0.13.2 - <https://codeberg.org/sarah-quinones/dyn-stack>
- `dyn-stack-macros` 0.1.3 - <https://github.com/kitegi/dynstack/>
- `ebur128` 0.1.11 - <https://github.com/sdroege/ebur128>
- `embed-resource` 3.0.11 - <https://github.com/nabijaczleweli/rust-embed-resource>
- `fancy-regex` 0.17.0 - <https://github.com/fancy-regex/fancy-regex>
- `fax` 0.2.7 - <https://github.com/pdf-rs/fax>
- `generic-array` 0.14.7 - <https://github.com/fizyk20/generic-array>
- `http-body` 1.0.1 - <https://github.com/hyperium/http-body>
- `http-body-util` 0.1.3 - <https://github.com/hyperium/http-body>
- `hyper` 1.10.1 - <https://github.com/hyperium/hyper>
- `hyper-util` 0.1.20 - <https://github.com/hyperium/hyper-util>
- `ico` 0.5.0 - <https://github.com/mdsteele/rust-ico>
- `infer` 0.19.0 - <https://github.com/bojand/infer>
- `libm` 0.2.16 - <https://github.com/rust-lang/compiler-builtins>
- `libsqlite3-sys` 0.30.1 - <https://github.com/rusqlite/rusqlite>
- `matchers` 0.2.0 - <https://github.com/hawkw/matchers>
- `metrics` 0.24.6 - <https://github.com/metrics-rs/metrics>
- `mio` 1.2.1 - <https://github.com/tokio-rs/mio>
- `new_debug_unreachable` 1.0.6 - <https://github.com/mbrubeck/rust-debug-unreachable>
- `nom` 7.1.3 - <https://github.com/Geal/nom>
- `nom` 8.0.0 - <https://github.com/rust-bakery/nom>
- `nom-language` 0.1.0 - <https://github.com/rust-bakery/nom>
- `nu-ansi-term` 0.50.3 - <https://github.com/nushell/nu-ansi-term>
- `phf` 0.13.1 - <https://github.com/rust-phf/rust-phf>
- `phf_codegen` 0.13.1 - <https://github.com/rust-phf/rust-phf>
- `phf_generator` 0.13.1 - <https://github.com/rust-phf/rust-phf>
- `phf_macros` 0.13.1 - <https://github.com/rust-phf/rust-phf>
- `phf_shared` 0.13.1 - <https://github.com/rust-phf/rust-phf>
- `plist` 1.9.0 - <https://github.com/ebarnard/rust-plist/>
- `precomputed-hash` 0.1.1 - <https://github.com/emilio/precomputed-hash>
- `pulp` 0.22.3 - <https://github.com/sarah-quinones/pulp/>
- `pulp-wasm-simd-flag` 0.1.1 - <https://github.com/sarah-quinones/pulp/>
- `quick-xml` 0.39.4 - <https://github.com/tafia/quick-xml>
- `raw-cpuid` 11.6.0 - <https://github.com/gz/rust-cpuid>
- `reborrow` 0.5.5 - <https://github.com/sarah-ek/reborrow/>
- `rusqlite` 0.32.1 - <https://github.com/rusqlite/rusqlite>
- `schemars` 0.8.22 - <https://github.com/GREsau/schemars>
- `schemars` 0.9.0 - <https://github.com/GREsau/schemars>
- `schemars` 1.2.1 - <https://github.com/GREsau/schemars>
- `schemars_derive` 0.8.22 - <https://github.com/GREsau/schemars>
- `sharded-slab` 0.1.7 - <https://github.com/hawkw/sharded-slab>
- `simd-adler32` 0.3.9 - <https://github.com/mcountryman/simd-adler32>
- `slab` 0.4.12 - <https://github.com/tokio-rs/slab>
- `strsim` 0.11.1 - <https://github.com/rapidfuzz/strsim-rs>
- `synstructure` 0.13.2 - <https://github.com/mystor/synstructure>
- `tauri-winres` 0.3.6 - <https://github.com/tauri-apps/winres>
- `tiff` 0.11.3 - <https://github.com/image-rs/image-tiff>
- `tokio` 1.52.3 - <https://github.com/tokio-rs/tokio>
- `tower` 0.5.3 - <https://github.com/tower-rs/tower>
- `tower-http` 0.6.11 - <https://github.com/tower-rs/tower-http>
- `tower-layer` 0.3.3 - <https://github.com/tower-rs/tower>
- `tower-service` 0.3.3 - <https://github.com/tower-rs/tower>
- `tracing` 0.1.44 - <https://github.com/tokio-rs/tracing>
- `tracing-attributes` 0.1.31 - <https://github.com/tokio-rs/tracing>
- `tracing-core` 0.1.36 - <https://github.com/tokio-rs/tracing>
- `tracing-log` 0.2.0 - <https://github.com/tokio-rs/tracing>
- `tracing-subscriber` 0.3.23 - <https://github.com/tokio-rs/tracing>
- `transcribe-cpp` 0.4.2 - <https://github.com/handy-computer/transcribe.cpp>
- `transcribe-cpp-sys` 0.4.2 - <https://github.com/handy-computer/transcribe.cpp>
- `try-lock` 0.2.5 - <https://github.com/seanmonstar/try-lock>
- `urlpattern` 0.3.0 - <https://github.com/denoland/rust-urlpattern>
- `vswhom` 0.1.0 - <https://github.com/nabijaczleweli/vswhom.rs>
- `vswhom-sys` 0.1.3 - <https://github.com/nabijaczleweli/vswhom-sys.rs>
- `want` 0.3.1 - <https://github.com/seanmonstar/want>
- `webview2-com` 0.38.2 - <https://github.com/wravery/webview2-rs>
- `webview2-com-macros` 0.8.1 - <https://github.com/wravery/webview2-rs>
- `webview2-com-sys` 0.38.2 - <https://github.com/wravery/webview2-rs>
- `winnow` 0.7.15 - <https://github.com/winnow-rs/winnow>
- `winnow` 1.0.3 - <https://github.com/winnow-rs/winnow>
- `winreg` 0.55.0 - <https://github.com/gentoo90/winreg-rs>
- `xn-gemm` 0.20.2 - <https://github.com/gradium-ai/gemm/>
- `xn-gemm-c32` 0.20.2 - <https://github.com/gradium-ai/gemm/>
- `xn-gemm-c64` 0.20.2 - <https://github.com/gradium-ai/gemm/>
- `xn-gemm-common` 0.20.2 - <https://github.com/gradium-ai/gemm/>
- `xn-gemm-f16` 0.20.2 - <https://github.com/gradium-ai/gemm/>
- `xn-gemm-f32` 0.20.2 - <https://github.com/gradium-ai/gemm/>
- `xn-gemm-f64` 0.20.2 - <https://github.com/gradium-ai/gemm/>
- `zmij` 1.0.21 - <https://github.com/dtolnay/zmij>

### Unicode-3.0 (18 crates)

- `icu_collections` 2.2.0 - <https://github.com/unicode-org/icu4x>
- `icu_locale_core` 2.2.0 - <https://github.com/unicode-org/icu4x>
- `icu_normalizer` 2.2.0 - <https://github.com/unicode-org/icu4x>
- `icu_normalizer_data` 2.2.0 - <https://github.com/unicode-org/icu4x>
- `icu_properties` 2.2.0 - <https://github.com/unicode-org/icu4x>
- `icu_properties_data` 2.2.0 - <https://github.com/unicode-org/icu4x>
- `icu_provider` 2.2.0 - <https://github.com/unicode-org/icu4x>
- `litemap` 0.8.2 - <https://github.com/unicode-org/icu4x>
- `potential_utf` 0.1.5 - <https://github.com/unicode-org/icu4x>
- `tinystr` 0.8.3 - <https://github.com/unicode-org/icu4x>
- `writeable` 0.6.3 - <https://github.com/unicode-org/icu4x>
- `yoke` 0.8.3 - <https://github.com/unicode-org/icu4x>
- `yoke-derive` 0.8.2 - <https://github.com/unicode-org/icu4x>
- `zerofrom` 0.1.8 - <https://github.com/unicode-org/icu4x>
- `zerofrom-derive` 0.1.7 - <https://github.com/unicode-org/icu4x>
- `zerotrie` 0.2.4 - <https://github.com/unicode-org/icu4x>
- `zerovec` 0.11.6 - <https://github.com/unicode-org/icu4x>
- `zerovec-derive` 0.11.3 - <https://github.com/unicode-org/icu4x>

### Apache-2.0 (12 crates)

- `borsh-derive` 1.8.1 - <https://github.com/near/borsh-rs>
- `cpal` 0.15.3 - <https://github.com/rustaudio/cpal>
- `esaxx-rs` 0.1.10 - <https://github.com/Narsil/esaxx-rs>
- `hound` 3.5.1 - <https://github.com/ruuda/hound>
- `minijinja` 3.1.0 - <https://github.com/mitsuhiko/minijinja>
- `prost` 0.14.6 - <https://github.com/tokio-rs/prost>
- `prost-derive` 0.14.6 - <https://github.com/tokio-rs/prost>
- `safetensors` 0.7.0 - <https://github.com/huggingface/safetensors>
- `spm_precompiled` 0.1.4 - <https://github.com/huggingface/spm_precompiled>
- `sync_wrapper` 1.0.2 - <https://github.com/Actyx/sync_wrapper>
- `tao` 0.35.3 - <https://github.com/tauri-apps/tao>
- `tokenizers` 0.23.2 - <https://github.com/huggingface/tokenizers>

### Apache-2.0 OR MIT OR Zlib (10 crates)

- `bytemuck` 1.25.0 - <https://github.com/Lokathor/bytemuck>
- `bytemuck_derive` 1.12.1 - <https://github.com/Lokathor/bytemuck>
- `macro_rules_attribute` 0.2.3 - <https://github.com/danielhenrymantilla/macro_rules_attribute-rs>
- `macro_rules_attribute-proc_macro` 0.2.3 - <https://github.com/danielhenrymantilla/macro_rules_attribute-rs>
- `miniz_oxide` 0.8.9 - <https://github.com/Frommi/miniz_oxide/tree/master/miniz_oxide>
- `raw-window-handle` 0.6.2 - <https://github.com/rust-windowing/raw-window-handle>
- `tinyvec` 1.11.0 - <https://github.com/Lokathor/tinyvec>
- `tinyvec_macros` 0.1.1 - <https://github.com/Soveu/tinyvec_macros>
- `zune-core` 0.5.1 - <https://github.com/etemesi254/zune-image>
- `zune-jpeg` 0.5.15 - <https://github.com/etemesi254/zune-image/tree/dev/crates/zune-jpeg>

### BSD-3-Clause (9 crates)

- `alloc-no-stdlib` 2.0.4 - <https://github.com/dropbox/rust-alloc-no-stdlib>
- `alloc-stdlib` 0.2.4 - <https://github.com/dropbox/rust-alloc-no-stdlib>
- `sonora` 0.3.0 - <https://github.com/dignifiedquire/sonora>
- `sonora-aec3` 0.3.0 - <https://github.com/dignifiedquire/sonora>
- `sonora-agc2` 0.3.0 - <https://github.com/dignifiedquire/sonora>
- `sonora-common-audio` 0.3.0 - <https://github.com/dignifiedquire/sonora>
- `sonora-fft` 0.3.0 - <https://github.com/dignifiedquire/sonora>
- `sonora-ns` 0.3.0 - <https://github.com/dignifiedquire/sonora>
- `sonora-simd` 0.3.0 - <https://github.com/dignifiedquire/sonora>

### MIT OR Unlicense (7 crates)

- `aho-corasick` 1.1.4 - <https://github.com/BurntSushi/aho-corasick>
- `byteorder` 1.5.0 - <https://github.com/BurntSushi/byteorder>
- `byteorder-lite` 0.1.0 - <https://github.com/image-rs/byteorder-lite>
- `memchr` 2.8.2 - <https://github.com/BurntSushi/memchr>
- `same-file` 1.0.6 - <https://github.com/BurntSushi/same-file>
- `walkdir` 2.5.0 - <https://github.com/BurntSushi/walkdir>
- `winapi-util` 0.1.11 - <https://github.com/BurntSushi/winapi-util>

### MPL-2.0 (5 crates)

- `cssparser` 0.36.0 - <https://github.com/servo/rust-cssparser>
- `cssparser-macros` 0.6.1 - <https://github.com/servo/rust-cssparser>
- `dtoa-short` 0.3.5 - <https://github.com/upsuper/dtoa-short>
- `option-ext` 0.2.0 - <https://github.com/soc/option-ext>
- `selectors` 0.36.1 - <https://github.com/servo/stylo>

### Apache-2.0 OR BSD-2-Clause OR MIT (2 crates)

- `zerocopy` 0.8.52 - <https://github.com/google/zerocopy>
- `zerocopy-derive` 0.8.52 - <https://github.com/google/zerocopy>

### Apache-2.0 OR BSD-3-Clause (2 crates)

- `moxcms` 0.8.1 - <https://github.com/awxkee/moxcms>
- `pxfm` 0.1.29 - <https://github.com/awxkee/pxfm>

### BSL-1.0 (2 crates)

- `clipboard-win` 5.4.1 - <https://github.com/DoumanAsh/clipboard-win>
- `error-code` 3.3.2 - <https://github.com/DoumanAsh/error-code>

### (MIT OR Apache-2.0) AND Unicode-3.0 (1 crate)

- `unicode-ident` 1.0.24 - <https://github.com/dtolnay/unicode-ident>

### 0BSD OR Apache-2.0 OR MIT (1 crate)

- `adler2` 2.0.1 - <https://github.com/oyvindln/adler2>

### Apache-2.0 AND MIT (1 crate)

- `dpi` 0.1.2 - <https://github.com/rust-windowing/winit>

### Apache-2.0 OR BlueOak-1.0.0 OR MIT (1 crate)

- `anymap3` 1.1.0 - <https://github.com/reivilibre/anymap3>

### Apache-2.0 OR BSL-1.0 (1 crate)

- `ryu` 1.0.23 - <https://github.com/dtolnay/ryu>

### Apache-2.0 OR CC0-1.0 OR MIT-0 (1 crate)

- `dunce` 1.0.5 - <https://gitlab.com/kornelski/dunce>

### BSD-3-Clause AND MIT (1 crate)

- `brotli` 8.0.4 - <https://github.com/dropbox/rust-brotli>

### BSD-3-Clause OR MIT (1 crate)

- `brotli-decompressor` 5.0.3 - <https://github.com/dropbox/rust-brotli-decompressor>

### Zlib (1 crate)

- `foldhash` 0.2.0 - <https://github.com/orlp/foldhash>

## npm packages

5 production npm packages are bundled into the settings window, as locked in `package-lock.json`. Development tools (the compiler, test runner, linters and the Tauri CLI) are not shipped.

### MIT (3 packages)

- `react` 19.2.7 - <https://www.npmjs.com/package/react>
- `react-dom` 19.2.7 - <https://www.npmjs.com/package/react-dom>
- `scheduler` 0.27.0 - <https://www.npmjs.com/package/scheduler>

### Apache-2.0 OR MIT (1 package)

- `@tauri-apps/api` 2.11.1 - <https://www.npmjs.com/package/@tauri-apps/api>

### ISC (1 package)

- `lucide-react` 0.525.0 - <https://www.npmjs.com/package/lucide-react>

<!-- END GENERATED -->
