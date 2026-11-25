import { StatusPill } from "../components/ui/Buttons";
import { Panel } from "../components/ui/Panel";
import { describeHotkeys } from "../domain/hotkeys";
import type { DiagnosticMetric } from "../domain/types";

export function HotkeysScreen({
  diagnostics,
  conversationHotkey,
  readingHotkey,
}: {
  diagnostics: DiagnosticMetric[] | null;
  conversationHotkey: string | null | undefined;
  readingHotkey: string | null | undefined;
}) {
  const rows = describeHotkeys({ diagnostics, conversationHotkey, readingHotkey });
  return (
    <Panel
      title="Global hotkeys"
      wide
      description="These work in any program. They cannot be changed yet. One thing uses the microphone at a time: during a conversation the dictation and reading hotkeys are ignored."
    >
      <div className="table-scroll">
        <table className="data-table">
          <thead>
            <tr>
              <th scope="col">Keys</th>
              <th scope="col">What it does</th>
              <th scope="col">Status</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((row) => (
              <tr key={row.keys}>
                <th scope="row">
                  <kbd>{row.keys}</kbd>
                </th>
                <td>{row.action}</td>
                <td>
                  <StatusPill tone={row.tone}>{row.status}</StatusPill>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </Panel>
  );
}
