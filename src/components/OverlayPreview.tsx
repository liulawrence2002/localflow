import { AlertCircle, CheckCircle2, Loader2, Mic, WandSparkles } from "lucide-react";
import type { DictationPhase } from "../domain/types";

interface OverlayPreviewProps {
  phase: DictationPhase;
  message?: string;
}

const overlayIcon = {
  Idle: Mic,
  Preparing: Loader2,
  Listening: Mic,
  Transcribing: Loader2,
  Refining: WandSparkles,
  Inserting: Loader2,
  Complete: CheckCircle2,
  Cancelled: AlertCircle,
  Error: AlertCircle,
};

export function OverlayPreview({ phase, message }: OverlayPreviewProps) {
  const Icon = overlayIcon[phase];
  return (
    <div className={`overlay-preview overlay-preview--${phase.toLowerCase()}`} aria-live="polite">
      <Icon aria-hidden="true" size={18} />
      <span>{message ?? phase}</span>
    </div>
  );
}
