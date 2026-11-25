# LocalFlow Documentation

Reference and design documentation for LocalFlow. Start with the [project README](../README.md) for installing and a summary.

## Guides

- [USER_GUIDE.md](USER_GUIDE.md) — installing, the models, dictation, conversation, reading pages aloud, voice commands for coding agents, settings and troubleshooting.
- [MODEL_SETUP.md](MODEL_SETUP.md) — every model LocalFlow can use (speech recognition, the voices, voice detection, end of turn, Ollama), the scripts that download them, where they go, and how they are chosen.
- [DEVELOPMENT.md](DEVELOPMENT.md) — contributor setup, the runtime files, CPU and GPU builds, tests, benchmarks, releases and extension points.
- [ARCHITECTURE.md](ARCHITECTURE.md) — system design, components, and data flow.
- [TESTING.md](TESTING.md) — the automated tests, ignored tests, manual checklists and the latest verified results.
- [mobile-sdk.md](mobile-sdk.md) — the experimental `@localflow/sdk` package for embedding the voice state and overlay in other React apps.

## Research & Benchmarks

- [VOICE_ASSISTANT_PLAN.md](VOICE_ASSISTANT_PLAN.md) — audit, measured baselines, technology choices, milestones and the [roadmap](VOICE_ASSISTANT_PLAN.md#roadmap) for the conversation mode, reading aloud and coding-agent commands.
- [research/VOICE_PIPELINE_REVIEW.md](research/VOICE_PIPELINE_REVIEW.md) — audit of the voice pipeline, survey of speech models, and decisions (a snapshot dated 2026-11-22).
- [benchmarks/CONVERSATION_BENCHMARK.md](benchmarks/CONVERSATION_BENCHMARK.md) — measured end-to-end latency of conversation mode, what was found, and how to reproduce it.
- [benchmarks/ASR_SPEED_BENCHMARK.md](benchmarks/ASR_SPEED_BENCHMARK.md) — CPU thread count and GPU (Vulkan) speech recognition.
- [benchmarks/TURN_JUDGE_BENCHMARK.md](benchmarks/TURN_JUDGE_BENCHMARK.md) — the text turn judge against Smart Turn on Smart Turn's public test set.
- [benchmarks/ASR_BENCHMARK.md](benchmarks/ASR_BENCHMARK.md) — before-and-after speech-to-text measurements and how to reproduce them.
- [research/appendix/](research/appendix/) — the underlying research reports with sources (snapshots dated 2026-11-22).

## Policy & Status

- [PRIVACY.md](PRIVACY.md) — data handling and privacy controls.
- [network-policy.md](network-policy.md) — what the app may connect to, and the one exception.
- [development/PLAN.md](development/PLAN.md) — what each change built, with known limitations and next risks.
- [../CHANGELOG.md](../CHANGELOG.md) — notable changes.

> The security policy, including how to report a vulnerability, is at the repository root in [SECURITY.md](../SECURITY.md). Licences: [LICENSE-MIT](../LICENSE-MIT), [LICENSE-APACHE](../LICENSE-APACHE) and [THIRD_PARTY_NOTICES.md](../THIRD_PARTY_NOTICES.md).

## Archive

Historical snapshots from July 2026, kept for reference. They may not match the current code.

- [archive/REPO_AUDIT.md](archive/REPO_AUDIT.md) — the evidence-based audit taken before the July upgrade.
- [archive/PARITY_MATRIX.md](archive/PARITY_MATRIX.md) — capability-by-capability parity status and the benchmark plan from that audit.
- [archive/handoff.md](archive/handoff.md) — engineering handoff notes for the July upgrade.
- [archive/prompts/](archive/prompts/) — early prompt contracts; no code reads them.
