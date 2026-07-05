import { createLocalFlowClient, createMockTranscriber } from "@localflow/sdk";
import { LocalFlowOverlay, LocalFlowProvider, useLocalFlow } from "@localflow/sdk/react";
import { useEffect, useState } from "react";

const sampleText = "LocalFlow mobile SDK inserted this text from an in-app dictation provider.";

export function MobileSdkExample() {
  const [client] = useState(() =>
    createLocalFlowClient({
      transcriber: createMockTranscriber({
        text: sampleText,
        refinedText: `${sampleText} The host app can choose whether to use quick or refined text.`,
        delayMs: 450,
      }),
    }),
  );

  return (
    <LocalFlowProvider client={client}>
      <MobileSdkExampleInner />
    </LocalFlowProvider>
  );
}

function MobileSdkExampleInner() {
  const { client, status, voiceState, error } = useLocalFlow();
  const [text, setText] = useState("");
  const isListening = voiceState.phase === "listening";
  const isBusy = ["listening", "processing", "refining"].includes(voiceState.phase);

  useEffect(() => {
    return client.on("transcript", (event) => {
      if (event.kind !== "quick") {
        return;
      }

      setText((current) => `${current}${current ? "\n\n" : ""}${event.text}`);
    });
  }, [client]);

  async function toggleDictation() {
    if (isListening) {
      await client.stopDictation();
      return;
    }

    await client.startDictation({
      context: {
        surface: "mobile-sdk-example",
        field: "notes",
      },
    });
  }

  return (
    <main className="mobile-sdk-example">
      <section className="mobile-sdk-example__phone" aria-label="LocalFlow mobile SDK example">
        <header className="mobile-sdk-example__header">
          <div>
            <span>LocalFlow SDK</span>
            <h1>Notes</h1>
          </div>
          <strong>{status?.phase ?? "idle"}</strong>
        </header>

        <textarea
          aria-label="Mobile host app text field"
          value={text}
          onChange={(event) => setText(event.currentTarget.value)}
          placeholder="Tap Dictate, speak in the host app, then stop to insert text."
        />

        <footer className="mobile-sdk-example__actions">
          <button
            type="button"
            onClick={toggleDictation}
            disabled={voiceState.phase === "processing"}
            title={isListening ? "Stop dictation" : "Start dictation"}
          >
            {isListening ? "Stop" : "Dictate"}
          </button>
          <button
            type="button"
            onClick={() => void client.cancelDictation()}
            disabled={!isBusy}
            title="Cancel dictation"
          >
            Cancel
          </button>
        </footer>

        {error ? <p className="mobile-sdk-example__error">{error.message}</p> : null}
        <LocalFlowOverlay state={voiceState} placement="in-app" hiddenWhenIdle />
      </section>
    </main>
  );
}
