# LocalFlow Documentation

Reference and design documentation for LocalFlow. Start with the [project README](../README.md) for setup and usage.

## Guides

- [USER_GUIDE.md](USER_GUIDE.md) — using LocalFlow: dictation, conversation, reading pages aloud, voice commands for coding agents, settings and troubleshooting.
- [DEVELOPMENT.md](DEVELOPMENT.md) — contributor setup, CPU and GPU builds, tests, benchmarks and extension points.
- [ARCHITECTURE.md](ARCHITECTURE.md) — system design, components, and data flow.
- [MODEL_SETUP.md](MODEL_SETUP.md) — Whisper and Ollama model configuration.
- [TESTING.md](TESTING.md) — how to run and interpret the test suites.
- [TROUBLESHOOTING.md](TROUBLESHOOTING.md) — common issues and fixes.

## Research & Benchmarks

- [VOICE_ASSISTANT_PLAN.md](VOICE_ASSISTANT_PLAN.md) — audit, measured baselines, technology choices and milestones for the hands-free conversation mode and screen-aware reading.
- [research/VOICE_PIPELINE_REVIEW.md](research/VOICE_PIPELINE_REVIEW.md) — audit of the voice pipeline, survey of current speech models, decisions, and roadmap.
- [benchmarks/CONVERSATION_BENCHMARK.md](benchmarks/CONVERSATION_BENCHMARK.md) — measured end-to-end latency of conversation mode, what was found, and how to reproduce it.
- [benchmarks/ASR_SPEED_BENCHMARK.md](benchmarks/ASR_SPEED_BENCHMARK.md) — CPU thread count and GPU (Vulkan) speech recognition.
- [benchmarks/TURN_JUDGE_BENCHMARK.md](benchmarks/TURN_JUDGE_BENCHMARK.md) — the text turn judge against Smart Turn on Smart Turn's public test set.
- [benchmarks/ASR_BENCHMARK.md](benchmarks/ASR_BENCHMARK.md) — before-and-after speech-to-text measurements and how to reproduce them.
- [research/appendix/](research/appendix/) — the underlying research reports with sources.

## Policy & Status

- [PRIVACY.md](PRIVACY.md) — data handling and privacy controls.
- [network-policy.md](network-policy.md) — outbound network allowlist policy.
- [ROADMAP.md](ROADMAP.md) — planned work and milestone status.

> Security policy lives at the repository root in [SECURITY.md](../SECURITY.md).

## Internal Notes

Working artifacts from the spec-driven upgrade; useful for contributors, not required for users.

- [REPO_AUDIT.md](REPO_AUDIT.md) — evidence-based audit of current state.
- [PARITY_MATRIX.md](PARITY_MATRIX.md) — capability-by-capability parity status.
- [development/PLAN.md](development/PLAN.md) — per-slice implementation plan.
- [development/handoff.md](development/handoff.md) — engineering handoff notes.
