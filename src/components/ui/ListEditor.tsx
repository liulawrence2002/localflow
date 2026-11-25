import { Plus } from "lucide-react";
import {
  useId,
  useRef,
  useState,
  type ChangeEvent,
  type CSSProperties,
  type ReactNode,
} from "react";
import { requiredText } from "../../domain/validation";
import { RemoveButton } from "./Buttons";
import { TextField, Toggle } from "./Fields";

type StringKey<T> = { [K in keyof T]-?: T[K] extends string ? K : never }[keyof T];
type BooleanKey<T> = { [K in keyof T]-?: T[K] extends boolean ? K : never }[keyof T];

/** One column of a list: a text field that must not be empty, or an on/off checkbox. */
export type ListColumn<T> =
  | {
      kind: "text" | "multiline";
      key: StringKey<T>;
      label: string;
      /** Shown when the field is left empty. */
      requiredMessage: string;
      placeholder?: string;
    }
  | { kind: "toggle"; key: BooleanKey<T>; label: string };

export interface ListEditorProps<T extends { id: string }> {
  title: string;
  description: ReactNode;
  /** "replacement", for "Add replacement". */
  itemNoun: string;
  columns: ReadonlyArray<ListColumn<Omit<T, "id">>>;
  items: T[];
  emptyText: string;
  createDraft: () => Omit<T, "id">;
  /** The words that name a row in accessible labels, for example its phrase. */
  nameOf: (item: Omit<T, "id">) => string;
  onAdd: (draft: Omit<T, "id">) => void;
  onUpdate: (id: string, patch: Partial<Omit<T, "id">>) => void;
  onRemove: (id: string) => void;
}

function textOf<T>(item: T, key: keyof T): string {
  const value = item[key];
  return typeof value === "string" ? value : "";
}

function patchOf<T>(key: keyof T, value: T[keyof T]): Partial<T> {
  const patch: Partial<T> = {};
  patch[key] = value;
  return patch;
}

/** Grid columns for the rows (ending in the remove button) or the add form (the Add button). */
function columnTemplate(columns: ReadonlyArray<{ kind: string }>, last: string): string {
  return [
    ...columns.map((column) =>
      column.kind === "toggle"
        ? "3.5rem"
        : column.kind === "multiline"
          ? "minmax(200px, 1.6fr)"
          : "minmax(140px, 1fr)",
    ),
    last,
  ].join(" ");
}

/**
 * A list of settings rows with an add form above it. Text is edited locally and committed when
 * the user leaves a field, so typing is never trimmed and a row is never deleted because a
 * field was emptied; an empty required field shows an error instead.
 */
export function ListEditor<T extends { id: string }>({
  title,
  description,
  itemNoun,
  columns,
  items,
  emptyText,
  createDraft,
  nameOf,
  onAdd,
  onUpdate,
  onRemove,
}: ListEditorProps<T>) {
  type Draft = Omit<T, "id">;
  const headingId = useId();
  const formId = useId();
  const [draft, setDraft] = useState<Draft>(createDraft);
  const [errors, setErrors] = useState<Partial<Record<keyof Draft, string>>>({});
  const firstInputRef = useRef<HTMLInputElement | null>(null);
  const template = {
    "--list-columns": columnTemplate(columns, "var(--control-height)"),
    "--list-add-columns": columnTemplate(columns, "max-content"),
  } as CSSProperties;

  function add() {
    const nextErrors: Partial<Record<keyof Draft, string>> = {};
    let cleaned = draft;
    for (const column of columns) {
      if (column.kind === "toggle") {
        continue;
      }
      const text = requiredText(textOf(draft, column.key));
      if (text === null) {
        nextErrors[column.key] = column.requiredMessage;
      } else {
        cleaned = { ...cleaned, ...patchOf<Draft>(column.key, text as Draft[keyof Draft]) };
      }
    }
    setErrors(nextErrors);
    if (Object.keys(nextErrors).length > 0) {
      return;
    }
    onAdd(cleaned);
    setDraft(createDraft());
    firstInputRef.current?.focus();
  }

  return (
    <section className="panel panel--wide" aria-labelledby={headingId}>
      <div className="panel-heading">
        <h2 id={headingId}>{title}</h2>
      </div>
      <p className="field-hint panel-description">{description}</p>

      <form
        className="list-add"
        style={template}
        aria-label={`Add ${itemNoun}`}
        onSubmit={(event) => {
          event.preventDefault();
          add();
        }}
      >
        {columns.map((column, index) => {
          const inputId = `${formId}-${String(column.key)}`;
          if (column.kind === "toggle") {
            const checked = Boolean(draft[column.key]);
            return (
              <div className="list-add__toggle" key={String(column.key)}>
                <Toggle
                  label={column.label}
                  checked={checked}
                  onChange={(value) =>
                    setDraft((current) => ({
                      ...current,
                      ...patchOf<Draft>(column.key, value as Draft[keyof Draft]),
                    }))
                  }
                />
              </div>
            );
          }
          const error = errors[column.key];
          const errorId = `${inputId}-error`;
          const inputProps = {
            id: inputId,
            value: textOf(draft, column.key),
            placeholder: column.placeholder,
            "aria-invalid": error ? true : undefined,
            "aria-describedby": error ? errorId : undefined,
            onChange: (event: ChangeEvent<HTMLInputElement | HTMLTextAreaElement>) => {
              const text = event.currentTarget.value;
              setDraft((current) => ({
                ...current,
                ...patchOf<Draft>(column.key, text as Draft[keyof Draft]),
              }));
            },
          };
          return (
            <div className="field" key={String(column.key)}>
              <label className="field-label" htmlFor={inputId}>
                {column.label}
              </label>
              {column.kind === "multiline" ? (
                <textarea {...inputProps} rows={3} />
              ) : (
                <input {...inputProps} ref={index === 0 ? firstInputRef : undefined} />
              )}
              {error ? (
                <p className="field-hint field-hint--error" id={errorId} role="alert">
                  {error}
                </p>
              ) : null}
            </div>
          );
        })}
        <button type="submit" className="list-add__button">
          <Plus size={16} aria-hidden="true" />
          Add
        </button>
      </form>

      {items.length === 0 ? (
        <p className="empty-state">{emptyText}</p>
      ) : (
        <div className="list-rows" style={template}>
          <div className="list-row list-row--header" aria-hidden="true">
            {columns.map((column) => (
              <span key={String(column.key)}>{column.label}</span>
            ))}
            <span />
          </div>
          <ul className="list-rows__items" aria-label={title}>
            {items.map((item) => {
              const name = nameOf(item);
              return (
                <li className="list-row" key={item.id}>
                  {columns.map((column) =>
                    column.kind === "toggle" ? (
                      <Toggle
                        key={String(column.key)}
                        label={`${column.label} for "${name}"`}
                        hideLabel
                        checked={Boolean(item[column.key])}
                        onChange={(value) =>
                          onUpdate(item.id, patchOf<Draft>(column.key, value as Draft[keyof Draft]))
                        }
                      />
                    ) : (
                      <TextField
                        key={String(column.key)}
                        label={`${column.label} for "${name}"`}
                        hideLabel
                        multiline={column.kind === "multiline"}
                        value={textOf(item, column.key)}
                        requiredMessage={column.requiredMessage}
                        onCommit={(value) =>
                          onUpdate(item.id, patchOf<Draft>(column.key, value as Draft[keyof Draft]))
                        }
                      />
                    ),
                  )}
                  <RemoveButton label={`Remove "${name}"`} onClick={() => onRemove(item.id)} />
                </li>
              );
            })}
          </ul>
        </div>
      )}
    </section>
  );
}
