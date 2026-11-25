import { fireEvent, render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";
import { NumberField, SelectField, TextField } from "../src/components/ui/Fields";
import { ListEditor } from "../src/components/ui/ListEditor";
import { defaultSettings } from "../src/domain/defaults";
import { updateReplacementRule } from "../src/domain/settings";
import type { ReplacementRule } from "../src/domain/types";

describe("NumberField", () => {
  function renderField(onCommit = vi.fn()) {
    render(
      <NumberField
        label="End-of-speech wait (ms)"
        value={900}
        min={300}
        max={3000}
        unit="ms"
        onCommit={onCommit}
      />,
    );
    return { input: screen.getByLabelText("End-of-speech wait (ms)"), onCommit };
  }

  it("commits a valid number only when the field is left", () => {
    const { input, onCommit } = renderField();

    fireEvent.change(input, { target: { value: "1200" } });
    expect(onCommit).not.toHaveBeenCalled();
    fireEvent.blur(input);

    expect(onCommit).toHaveBeenCalledWith(1200);
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("clamps an out-of-range number and says so", () => {
    const { input, onCommit } = renderField();

    fireEvent.change(input, { target: { value: "5000" } });
    fireEvent.blur(input);

    expect(onCommit).toHaveBeenCalledWith(3000);
    expect(screen.getByRole("alert")).toHaveTextContent("Set to 3000 ms");
  });

  it("refuses an empty value, keeps the saved one and never commits NaN", () => {
    const { input, onCommit } = renderField();

    fireEvent.change(input, { target: { value: "" } });
    fireEvent.blur(input);

    expect(onCommit).not.toHaveBeenCalled();
    expect(input).toHaveValue(900);
    expect(screen.getByRole("alert")).toHaveTextContent("Enter a whole number from 300 to 3000 ms");
    expect(input).toHaveAttribute("aria-invalid", "true");
  });

  it("commits on Enter", () => {
    const { input, onCommit } = renderField();

    fireEvent.change(input, { target: { value: "450" } });
    fireEvent.keyDown(input, { key: "Enter" });

    expect(onCommit).toHaveBeenCalledWith(450);
  });
});

describe("TextField", () => {
  it("keeps typed text, spaces included, until the field is left, then trims its ends", () => {
    const onCommit = vi.fn();
    render(
      <TextField label="Phrase" value="pie" onCommit={onCommit} requiredMessage="Enter it." />,
    );
    const input = screen.getByLabelText("Phrase");

    fireEvent.change(input, { target: { value: "pie " } });
    expect(input).toHaveValue("pie ");
    fireEvent.change(input, { target: { value: "pie torch " } });
    expect(input).toHaveValue("pie torch ");
    expect(onCommit).not.toHaveBeenCalled();

    fireEvent.blur(input);
    expect(onCommit).toHaveBeenCalledWith("pie torch");
  });

  it("refuses an empty required value and restores on Escape", () => {
    const onCommit = vi.fn();
    render(
      <TextField
        label="Phrase"
        value="PyTorch"
        onCommit={onCommit}
        requiredMessage="Enter a word."
      />,
    );
    const input = screen.getByLabelText("Phrase");

    fireEvent.change(input, { target: { value: "  " } });
    fireEvent.blur(input);
    expect(onCommit).not.toHaveBeenCalled();
    expect(screen.getByRole("alert")).toHaveTextContent("Enter a word.");

    fireEvent.keyDown(input, { key: "Escape" });
    expect(input).toHaveValue("PyTorch");
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });
});

describe("SelectField", () => {
  it("reports the chosen option's typed value", () => {
    const onChange = vi.fn<(value: "a" | "b") => void>();
    render(
      <SelectField
        label="Choice"
        value="a"
        options={[
          { value: "a", label: "First" },
          { value: "b", label: "Second" },
        ]}
        onChange={onChange}
      />,
    );

    fireEvent.change(screen.getByLabelText("Choice"), { target: { value: "b" } });

    expect(onChange).toHaveBeenCalledWith("b");
  });
});

/** The Replacements list, wired to the real settings mutations. */
function ReplacementsHarness({ onUpdate }: { onUpdate: (rules: ReplacementRule[]) => void }) {
  const [settings, setSettings] = useState(defaultSettings);
  return (
    <ListEditor<ReplacementRule>
      title="Replacements"
      description="Test"
      itemNoun="replacement"
      columns={[
        {
          kind: "text",
          key: "incorrect",
          label: "Heard",
          requiredMessage: "Enter what dictation hears.",
        },
        { kind: "text", key: "correct", label: "Write", requiredMessage: "Enter what to write." },
        { kind: "toggle", key: "enabled", label: "On" },
      ]}
      items={settings.replacements}
      emptyText="None"
      createDraft={() => ({ incorrect: "", correct: "", enabled: true })}
      nameOf={(rule) => rule.incorrect}
      onAdd={() => undefined}
      onUpdate={(id, patch) => {
        const next = updateReplacementRule(settings, id, patch);
        setSettings(next);
        onUpdate(next.replacements);
      }}
      onRemove={() => undefined}
    />
  );
}

describe("ListEditor row editing", () => {
  it("names each row's controls after the row, matching the visible labels", () => {
    render(<ReplacementsHarness onUpdate={vi.fn()} />);

    expect(screen.getByLabelText('Heard for "pie torch"')).toBeInTheDocument();
    expect(screen.getByLabelText('Write for "pie torch"')).toBeInTheDocument();
    expect(screen.getByLabelText('On for "pie torch"')).toBeChecked();
    expect(screen.getByRole("button", { name: 'Remove "pie torch"' })).toBeInTheDocument();
    // The add form uses the same words.
    expect(screen.getByLabelText("Heard")).toBeInTheDocument();
    expect(screen.getByLabelText("Write")).toBeInTheDocument();
  });

  it("lets a phrase be selected, deleted and retyped without losing the row", () => {
    const onUpdate = vi.fn();
    render(<ReplacementsHarness onUpdate={onUpdate} />);
    const field = screen.getByLabelText('Write for "pie torch"');

    // Select all and delete: the row stays and nothing is saved.
    fireEvent.change(field, { target: { value: "" } });
    fireEvent.blur(field);
    expect(onUpdate).not.toHaveBeenCalled();
    expect(screen.getByRole("alert")).toHaveTextContent("Enter what to write.");
    expect(screen.getByRole("button", { name: 'Remove "pie torch"' })).toBeInTheDocument();

    // Retype, with a space in the middle.
    fireEvent.change(field, { target: { value: "Py Torch" } });
    fireEvent.blur(field);
    expect(onUpdate).toHaveBeenLastCalledWith([
      expect.objectContaining({
        id: "replace-pytorch",
        incorrect: "pie torch",
        correct: "Py Torch",
      }),
    ]);
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("refuses to add a row with an empty required field", () => {
    const onAdd = vi.fn();
    render(
      <ListEditor<ReplacementRule>
        title="Replacements"
        description="Test"
        itemNoun="replacement"
        columns={[
          {
            kind: "text",
            key: "incorrect",
            label: "Heard",
            requiredMessage: "Enter what dictation hears.",
          },
          { kind: "text", key: "correct", label: "Write", requiredMessage: "Enter what to write." },
        ]}
        items={[]}
        emptyText="None yet."
        createDraft={() => ({ incorrect: "", correct: "", enabled: true })}
        nameOf={(rule) => rule.incorrect}
        onAdd={onAdd}
        onUpdate={() => undefined}
        onRemove={() => undefined}
      />,
    );

    fireEvent.change(screen.getByLabelText("Heard"), { target: { value: " teh " } });
    fireEvent.click(screen.getByRole("button", { name: "Add" }));
    expect(onAdd).not.toHaveBeenCalled();
    expect(screen.getByRole("alert")).toHaveTextContent("Enter what to write.");

    fireEvent.change(screen.getByLabelText("Write"), { target: { value: "the" } });
    fireEvent.click(screen.getByRole("button", { name: "Add" }));
    expect(onAdd).toHaveBeenCalledWith({ incorrect: "teh", correct: "the", enabled: true });
    expect(screen.getByLabelText("Heard")).toHaveValue("");
  });
});
