# LocalFlow Mobile SDK Spike

This branch introduces `@localflow/sdk` as an in-app SDK surface for Tauri/React mobile hosts. It reuses the LocalFlow waveform and event protocol without attempting system-wide mobile dictation.

## Current Shape

- `@localflow/sdk` exports the client, event types, and mock transcriber.
- `@localflow/sdk/react` exports `LocalFlowProvider`, `useLocalFlow`, and `LocalFlowOverlay`.
- `@localflow/sdk/adapters/tauri` contains a Tauri command/event adapter skeleton for mobile plugin work.
- `?view=mobile-sdk-example` opens a small host-app example that inserts mock transcript text into a textarea.

## Minimal Host Usage

```tsx
import { createLocalFlowClient, createMockTranscriber } from "@localflow/sdk";
import { LocalFlowOverlay, LocalFlowProvider, useLocalFlow } from "@localflow/sdk/react";

const client = createLocalFlowClient({
  transcriber: createMockTranscriber({ text: "Hello from LocalFlow mobile." }),
});

function Notes() {
  const { client, voiceState } = useLocalFlow();

  return (
    <>
      <button onClick={() => client.startDictation()}>Dictate</button>
      <button onClick={() => client.stopDictation()}>Stop</button>
      <LocalFlowOverlay state={voiceState} placement="in-app" hiddenWhenIdle />
    </>
  );
}

export function App() {
  return (
    <LocalFlowProvider client={client}>
      <Notes />
    </LocalFlowProvider>
  );
}
```

## Event Contract

- `voice_state`: `{ sessionId, phase, message, level, pitch, brightness }`
- `transcript`: `{ sessionId, text, kind, latencyMs }`
- `error`: `{ sessionId, code, message, recoverable }`
- `status`: SDK runtime status and diagnostics.

## Boundaries

The v1 spike is in-app only. Host apps decide where transcript text goes, and the SDK does not paste into other mobile apps. Mobile on-device Whisper should be added as a new `LocalFlowTranscriber` implementation after the SDK boundary is proven.

The SDK and `?view=mobile-sdk-example` route are supplementary developer surfaces. Normal desktop usage still launches from the Windows shortcut through `scripts\Start-LocalFlow.vbs`, then `scripts\Start-LocalFlow.ps1`, and finally the packaged `src-tauri\target\release\localflow.exe` without starting Vite or a localhost UI.
