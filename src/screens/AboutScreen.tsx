import { Panel, PanelGrid } from "../components/ui/Panel";

/** The version comes from package.json at build time (see `define` in vite.config.ts). */
const appVersion = __APP_VERSION__;

export function AboutScreen() {
  return (
    <PanelGrid>
      <Panel title="LocalFlow">
        <dl className="details">
          <div>
            <dt>Version</dt>
            <dd>{appVersion}</dd>
          </div>
          <div>
            <dt>Build</dt>
            <dd>{import.meta.env.DEV ? "Development" : "Release"}</dd>
          </div>
          <div>
            <dt>Licence</dt>
            <dd>MIT or Apache-2.0, at your option</dd>
          </div>
          <div>
            <dt>Copyright</dt>
            <dd>LocalFlow contributors</dd>
          </div>
        </dl>
      </Panel>
    </PanelGrid>
  );
}
