import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { VoiceOverlay } from "./components/VoiceOverlay";

const isOverlayView = new URLSearchParams(window.location.search).get("view") === "overlay";
document.documentElement.classList.toggle("voice-overlay-root", isOverlayView);
document.body.classList.toggle("voice-overlay-body", isOverlayView);

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>{isOverlayView ? <VoiceOverlay /> : <App />}</React.StrictMode>,
);
