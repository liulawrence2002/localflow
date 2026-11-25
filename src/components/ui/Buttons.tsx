import { Trash2 } from "lucide-react";
import type { ReactNode } from "react";
import type { StatusTone } from "../../domain/format";

/** A remove button. `label` must name the row, for example `Remove "pie torch"`. */
export function RemoveButton({ label, onClick }: { label: string; onClick: () => void }) {
  return (
    <button
      className="icon-button"
      type="button"
      onClick={onClick}
      title={label}
      aria-label={label}
    >
      <Trash2 size={16} aria-hidden="true" />
    </button>
  );
}

/** A small coloured label for a state. The words carry the meaning, not the colour. */
export function StatusPill({ tone, children }: { tone: StatusTone; children: ReactNode }) {
  return <span className={`status-pill status-pill--${tone}`}>{children}</span>;
}
