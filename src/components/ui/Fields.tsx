import { useId, useState, type KeyboardEvent, type ReactNode } from "react";
import { boundedIntegerMessage, parseBoundedInteger, requiredText } from "../../domain/validation";

interface FieldFrameProps {
  /** Visible label. When `hideLabel` is set it is used as the accessible name only. */
  label: string;
  hideLabel?: boolean;
  hint?: ReactNode;
  error?: string | null;
  inputId: string;
  hintId: string;
  errorId: string;
  children: ReactNode;
  inline?: boolean;
}

function FieldFrame({
  label,
  hideLabel = false,
  hint,
  error,
  inputId,
  hintId,
  errorId,
  children,
  inline = false,
}: FieldFrameProps) {
  return (
    <div className={inline ? "field field--inline" : "field"}>
      <label className={hideLabel ? "visually-hidden" : "field-label"} htmlFor={inputId}>
        {label}
      </label>
      {children}
      {hint ? (
        <p className="field-hint" id={hintId}>
          {hint}
        </p>
      ) : null}
      {error ? (
        <p className="field-hint field-hint--error" id={errorId} role="alert">
          {error}
        </p>
      ) : null}
    </div>
  );
}

function describedBy(hint: ReactNode, error: string | null | undefined, ids: [string, string]) {
  const parts = [hint ? ids[0] : null, error ? ids[1] : null].filter(Boolean);
  return parts.length ? parts.join(" ") : undefined;
}

export interface TextFieldProps {
  label: string;
  value: string;
  /**
   * Called with the trimmed text when the user leaves the field or presses Enter. Text is kept
   * exactly as typed until then, so it is never trimmed or removed mid-word.
   */
  onCommit: (value: string) => void;
  /** When set, empty text is refused with this message and the saved value is kept. */
  requiredMessage?: string;
  multiline?: boolean;
  rows?: number;
  placeholder?: string;
  hint?: ReactNode;
  hideLabel?: boolean;
  spellCheck?: boolean;
  disabled?: boolean;
}

/** A text field that edits locally and commits on blur or Enter. */
export function TextField({
  label,
  value,
  onCommit,
  requiredMessage,
  multiline = false,
  rows = 3,
  placeholder,
  hint,
  hideLabel,
  spellCheck,
  disabled,
}: TextFieldProps) {
  const inputId = useId();
  const hintId = `${inputId}-hint`;
  const errorId = `${inputId}-error`;
  // `null` while not being edited: the field then shows the saved value.
  const [draft, setDraft] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  function commit() {
    if (draft === null) {
      return;
    }
    const trimmed = requiredText(draft);
    if (trimmed === null && requiredMessage) {
      // Keep the saved value; the row is never deleted because a field was emptied.
      setError(requiredMessage);
      return;
    }
    setError(null);
    setDraft(null);
    const next = trimmed ?? "";
    if (next !== value) {
      onCommit(next);
    }
  }

  function onKeyDown(event: KeyboardEvent<HTMLInputElement | HTMLTextAreaElement>) {
    if (event.key === "Escape" && draft !== null) {
      setDraft(null);
      setError(null);
      return;
    }
    if (event.key === "Enter" && !multiline) {
      event.preventDefault();
      commit();
    }
  }

  const common = {
    id: inputId,
    value: draft ?? value,
    placeholder,
    disabled,
    spellCheck,
    "aria-invalid": error ? true : undefined,
    "aria-describedby": describedBy(hint, error, [hintId, errorId]),
    onBlur: commit,
    onKeyDown,
  };

  return (
    <FieldFrame
      label={label}
      hideLabel={hideLabel}
      hint={hint}
      error={error}
      inputId={inputId}
      hintId={hintId}
      errorId={errorId}
    >
      {multiline ? (
        <textarea
          {...common}
          rows={rows}
          onChange={(event) => setDraft(event.currentTarget.value)}
        />
      ) : (
        <input {...common} onChange={(event) => setDraft(event.currentTarget.value)} />
      )}
    </FieldFrame>
  );
}

export interface NumberFieldProps {
  label: string;
  value: number;
  min: number;
  max: number;
  /** Shown after numbers in messages, for example "ms". */
  unit?: string;
  onCommit: (value: number) => void;
  hint?: ReactNode;
  disabled?: boolean;
}

/**
 * A whole-number field. What is typed is checked when the user leaves the field or presses
 * Enter: out-of-range numbers are moved into range and said so, and anything that is not a
 * number is refused and the saved value kept.
 */
export function NumberField({
  label,
  value,
  min,
  max,
  unit = "",
  onCommit,
  hint,
  disabled,
}: NumberFieldProps) {
  const inputId = useId();
  const hintId = `${inputId}-hint`;
  const errorId = `${inputId}-error`;
  const [draft, setDraft] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);

  function commit() {
    if (draft === null) {
      return;
    }
    const result = parseBoundedInteger(draft, min, max);
    setDraft(null);
    setMessage(boundedIntegerMessage(result, min, max, unit));
    if (result.ok && result.value !== value) {
      onCommit(result.value);
    }
  }

  return (
    <FieldFrame
      label={label}
      hint={hint}
      error={message}
      inputId={inputId}
      hintId={hintId}
      errorId={errorId}
    >
      <input
        id={inputId}
        type="number"
        inputMode="numeric"
        min={min}
        max={max}
        step={1}
        className="number-input"
        value={draft ?? String(value)}
        disabled={disabled}
        aria-invalid={message ? true : undefined}
        aria-describedby={describedBy(hint, message, [hintId, errorId])}
        onChange={(event) => setDraft(event.currentTarget.value)}
        onBlur={commit}
        onKeyDown={(event) => {
          if (event.key === "Enter") {
            event.preventDefault();
            commit();
          }
        }}
      />
    </FieldFrame>
  );
}

export interface SelectOption<T extends string> {
  value: T;
  label: string;
}

export interface SelectFieldProps<T extends string> {
  label: string;
  value: T;
  options: ReadonlyArray<SelectOption<T>>;
  onChange: (value: T) => void;
  hint?: ReactNode;
  disabled?: boolean;
}

/** A drop-down whose value is one of `options`, typed without casts. */
export function SelectField<T extends string>({
  label,
  value,
  options,
  onChange,
  hint,
  disabled,
}: SelectFieldProps<T>) {
  const inputId = useId();
  const hintId = `${inputId}-hint`;
  return (
    <FieldFrame
      label={label}
      hint={hint}
      inputId={inputId}
      hintId={hintId}
      errorId={`${inputId}-error`}
    >
      <select
        id={inputId}
        value={value}
        disabled={disabled}
        aria-describedby={hint ? hintId : undefined}
        onChange={(event) => {
          const selected = event.currentTarget.value;
          const option = options.find((candidate) => candidate.value === selected);
          if (option) {
            onChange(option.value);
          }
        }}
      >
        {options.map((option) => (
          <option key={option.value} value={option.value}>
            {option.label}
          </option>
        ))}
      </select>
    </FieldFrame>
  );
}

export interface ToggleProps {
  label: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
  hint?: ReactNode;
  /** Hide the visible label (inside a table row whose column says what it is). */
  hideLabel?: boolean;
  disabled?: boolean;
}

/** A checkbox with its label. */
export function Toggle({ label, checked, onChange, hint, hideLabel, disabled }: ToggleProps) {
  const inputId = useId();
  const hintId = `${inputId}-hint`;
  return (
    <div className="toggle-field">
      <label className="toggle" htmlFor={inputId}>
        <input
          id={inputId}
          type="checkbox"
          checked={checked}
          disabled={disabled}
          aria-describedby={hint ? hintId : undefined}
          onChange={(event) => onChange(event.currentTarget.checked)}
        />
        <span className={hideLabel ? "visually-hidden" : undefined}>{label}</span>
      </label>
      {hint ? (
        <p className="field-hint" id={hintId}>
          {hint}
        </p>
      ) : null}
    </div>
  );
}
