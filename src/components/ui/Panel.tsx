import { useId, type ReactNode } from "react";

interface PanelProps {
  title: string;
  /** Buttons shown beside the heading. */
  actions?: ReactNode;
  /** Span the whole row of the grid. */
  wide?: boolean;
  /** A sentence under the heading saying what the panel is for. */
  description?: ReactNode;
  children?: ReactNode;
}

/** A titled card. Its heading names the region for assistive technology. */
export function Panel({ title, actions, wide = false, description, children }: PanelProps) {
  const headingId = useId();
  return (
    <section className={wide ? "panel panel--wide" : "panel"} aria-labelledby={headingId}>
      <div className="panel-heading">
        <h2 id={headingId}>{title}</h2>
        {actions ? <div className="panel-heading__actions">{actions}</div> : null}
      </div>
      {description ? <p className="field-hint panel-description">{description}</p> : null}
      {children}
    </section>
  );
}

/** The grid that lays panels out in columns. */
export function PanelGrid({ children, home = false }: { children: ReactNode; home?: boolean }) {
  return <div className={home ? "panel-grid panel-grid--home" : "panel-grid"}>{children}</div>;
}
