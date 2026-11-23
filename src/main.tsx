import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { VoiceOverlay } from "./components/VoiceOverlay";
import { MobileSdkExample } from "./examples/MobileSdkExample";
import { OverlayDemo } from "./examples/OverlayDemo";

const view = new URLSearchParams(window.location.search).get("view");
const isOverlayView = view === "overlay";
const isMobileSdkExampleView = view === "mobile-sdk-example";
const isOverlayDemoView = view === "overlay-demo";
document.documentElement.classList.toggle("voice-overlay-root", isOverlayView);
document.body.classList.toggle("voice-overlay-body", isOverlayView);

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    {isOverlayView ? (
      <VoiceOverlay />
    ) : isOverlayDemoView ? (
      <OverlayDemo />
    ) : isMobileSdkExampleView ? (
      <MobileSdkExample />
    ) : (
      <App />
    )}
  </React.StrictMode>,
);
