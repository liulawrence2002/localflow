import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { VoiceOverlay } from "./components/VoiceOverlay";
import { MobileSdkExample } from "./examples/MobileSdkExample";

const view = new URLSearchParams(window.location.search).get("view");
const isOverlayView = view === "overlay";
const isMobileSdkExampleView = view === "mobile-sdk-example";
document.documentElement.classList.toggle("voice-overlay-root", isOverlayView);
document.body.classList.toggle("voice-overlay-body", isOverlayView);

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    {isOverlayView ? <VoiceOverlay /> : isMobileSdkExampleView ? <MobileSdkExample /> : <App />}
  </React.StrictMode>,
);
