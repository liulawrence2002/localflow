import React from "react";
import ReactDOM from "react-dom/client";
import { App } from "./app/App";
import { VoiceOverlay } from "./components/VoiceOverlay";
import "./styles/tokens.css";
import "./styles/app.css";

const view = new URLSearchParams(window.location.search).get("view");
const isOverlayView = view === "overlay";
document.documentElement.classList.toggle("voice-overlay-root", isOverlayView);
document.body.classList.toggle("voice-overlay-body", isOverlayView);

const root = ReactDOM.createRoot(document.getElementById("root") as HTMLElement);

async function chooseView(): Promise<React.ReactNode> {
  if (isOverlayView) {
    return <VoiceOverlay />;
  }
  // The demo and SDK example pages exist only in development builds; a release build never
  // contains them, whatever the address says.
  if (import.meta.env.DEV && view === "overlay-demo") {
    const { OverlayDemo } = await import("./examples/OverlayDemo");
    return <OverlayDemo />;
  }
  if (import.meta.env.DEV && view === "mobile-sdk-example") {
    const { MobileSdkExample } = await import("./examples/MobileSdkExample");
    return <MobileSdkExample />;
  }
  return <App />;
}

void chooseView().then((node) => root.render(<React.StrictMode>{node}</React.StrictMode>));
