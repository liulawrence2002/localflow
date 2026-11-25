import { ClipboardList, Download, RefreshCw } from "lucide-react";
import { useState } from "react";
import { StatusPill } from "../components/ui/Buttons";
import { LiveMessage } from "../components/ui/Notice";
import { Panel, PanelGrid } from "../components/ui/Panel";
import { serializeDiagnosticsExport } from "../domain/diagnostics";
import { formatDiagnosticStatus } from "../domain/format";
import type { AppStatus } from "../domain/types";

export function DiagnosticsScreen({
  status,
  refreshStatus,
}: {
  status: AppStatus | null;
  refreshStatus: () => Promise<AppStatus | null>;
}) {
  const [exportText, setExportText] = useState("");
  const [message, setMessage] = useState<{ text: string; tone: "status" | "alert" } | null>(null);
  const [isWorking, setIsWorking] = useState(false);

  async function refresh() {
    setIsWorking(true);
    try {
      const next = await refreshStatus();
      setMessage(
        next
          ? { text: "Diagnostics refreshed.", tone: "status" }
          : { text: "Diagnostics could not be refreshed.", tone: "alert" },
      );
    } finally {
      setIsWorking(false);
    }
  }

  async function prepareExport() {
    setIsWorking(true);
    try {
      // Refreshed first, so the export describes now rather than when the window opened.
      const next = (await refreshStatus()) ?? status;
      if (!next) {
        setMessage({ text: "There is nothing to export yet.", tone: "alert" });
        return;
      }
      setExportText(serializeDiagnosticsExport(next));
      setMessage({ text: "Export prepared. Review it before sharing.", tone: "status" });
    } finally {
      setIsWorking(false);
    }
  }

  async function copyExport() {
    try {
      await navigator.clipboard.writeText(exportText);
      setMessage({ text: "Export copied to the clipboard.", tone: "status" });
    } catch {
      setMessage({
        text: "The export could not be copied. Select the text and press Ctrl+C.",
        tone: "alert",
      });
    }
  }

  const rows = status?.diagnostics ?? [];

  return (
    <PanelGrid>
      <Panel
        title="Diagnostics"
        wide
        actions={
          <button type="button" onClick={() => void refresh()} disabled={isWorking}>
            <RefreshCw size={16} aria-hidden="true" />
            Refresh
          </button>
        }
      >
        {rows.length === 0 ? (
          <p className="empty-state">No diagnostics yet.</p>
        ) : (
          <div className="table-scroll">
            <table className="data-table">
              <thead>
                <tr>
                  <th scope="col">Item</th>
                  <th scope="col">Value</th>
                  <th scope="col">Status</th>
                </tr>
              </thead>
              <tbody>
                {rows.map((metric, index) => {
                  const pill = formatDiagnosticStatus(metric.status);
                  return (
                    <tr key={`${index}:${metric.label}`}>
                      <th scope="row">{metric.label}</th>
                      <td className="data-table__value">{metric.value}</td>
                      <td>
                        <StatusPill tone={pill.tone}>{pill.label}</StatusPill>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        )}
      </Panel>

      <Panel
        title="Export"
        wide
        description="For troubleshooting. The export leaves out dictated text, model paths, your user folder and microphone names."
      >
        <div className="button-row">
          <button type="button" onClick={() => void prepareExport()} disabled={isWorking}>
            <Download size={16} aria-hidden="true" />
            Prepare export
          </button>
          <button type="button" onClick={() => void copyExport()} disabled={!exportText}>
            <ClipboardList size={16} aria-hidden="true" />
            Copy
          </button>
        </div>
        <LiveMessage message={message?.text ?? null} tone={message?.tone ?? "status"} />
        <textarea
          className="export-text"
          aria-label="Diagnostics export"
          value={exportText}
          readOnly
          rows={12}
          spellCheck={false}
          placeholder="Choose Prepare export."
        />
      </Panel>
    </PanelGrid>
  );
}
