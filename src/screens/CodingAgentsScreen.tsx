import { Plus, RefreshCw } from "lucide-react";
import { useEffect, useId, useState } from "react";
import { RemoveButton, StatusPill } from "../components/ui/Buttons";
import { Notice } from "../components/ui/Notice";
import { Panel, PanelGrid } from "../components/ui/Panel";
import {
  cleanFolderInput,
  describeAgent,
  examplePhrases,
  formatSpokenNames,
  parseSpokenNames,
  quickFolderProblem,
  type CodingAgentsStatus,
  type CodingProjectView,
  type SpokenNameSuggestion,
} from "../domain/codingAgents";
import { conversationHotkeys } from "../domain/hotkeys";
import {
  detectCodingAgents,
  errorMessage,
  isDesktopApp,
  listCodingProjects,
  removeCodingProject,
  saveCodingProject,
  suggestProjectSpokenNames,
} from "../services/localflowClient";

export function CodingAgentsScreen() {
  const [projects, setProjects] = useState<CodingProjectView[]>([]);
  const [agents, setAgents] = useState<CodingAgentsStatus | null>(null);
  const [detecting, setDetecting] = useState(true);
  const [draftPath, setDraftPath] = useState("");
  const [draftNames, setDraftNames] = useState("");
  const [namesEdited, setNamesEdited] = useState(false);
  const [suggestion, setSuggestion] = useState<{
    path: string;
    result: SpokenNameSuggestion;
  } | null>(null);
  const [error, setError] = useState("");
  const [isSaving, setIsSaving] = useState(false);
  const folderId = useId();
  const namesId = useId();
  const problemId = useId();

  useEffect(() => {
    let cancelled = false;
    listCodingProjects().then(
      (next) => {
        if (!cancelled) {
          setProjects(next);
        }
      },
      (reason: unknown) => {
        if (!cancelled) {
          setError(errorMessage(reason, "The projects could not be loaded."));
        }
      },
    );
    detectCodingAgents()
      .then(
        (next) => {
          if (!cancelled) {
            setAgents(next);
          }
        },
        (reason: unknown) => {
          if (!cancelled) {
            setError(errorMessage(reason, "Looking for coding agents failed."));
          }
        },
      )
      .finally(() => {
        if (!cancelled) {
          setDetecting(false);
        }
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const cleanedPath = cleanFolderInput(draftPath);
  const quickProblem = cleanedPath ? quickFolderProblem(cleanedPath) : null;

  // Ask the native layer about the folder once typing pauses: whether it exists, and what it
  // might be called aloud.
  useEffect(() => {
    if (!cleanedPath || quickFolderProblem(cleanedPath)) {
      return;
    }
    let cancelled = false;
    const timer = window.setTimeout(() => {
      suggestProjectSpokenNames(cleanedPath).then(
        (next) => {
          if (!cancelled) {
            setSuggestion({ path: cleanedPath, result: next });
          }
        },
        (reason: unknown) => {
          if (!cancelled) {
            setSuggestion({
              path: cleanedPath,
              result: {
                folder: null,
                folderName: "",
                suggestions: [],
                problem: errorMessage(reason, "The folder could not be checked."),
              },
            });
          }
        },
      );
    }, 250);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [cleanedPath]);

  // A suggestion is only used while it is for the path currently typed.
  const currentSuggestion =
    suggestion && !quickProblem && suggestion.path === cleanedPath ? suggestion.result : null;
  const folderProblem = quickProblem ?? currentSuggestion?.problem ?? null;
  const namesValue = namesEdited
    ? draftNames
    : formatSpokenNames(currentSuggestion?.suggestions ?? []);
  const canAdd = Boolean(currentSuggestion?.folder) && !folderProblem && !isSaving;

  async function addProject() {
    setIsSaving(true);
    setError("");
    try {
      const next = await saveCodingProject({
        path: cleanedPath,
        spokenNames: parseSpokenNames(namesValue),
      });
      setProjects(next);
      setDraftPath("");
      setDraftNames("");
      setNamesEdited(false);
      setSuggestion(null);
    } catch (reason) {
      setError(errorMessage(reason, "The project could not be saved."));
    } finally {
      setIsSaving(false);
    }
  }

  async function checkAgain() {
    setDetecting(true);
    try {
      setAgents(await detectCodingAgents());
    } catch (reason) {
      setError(errorMessage(reason, "Looking for coding agents failed."));
    } finally {
      setDetecting(false);
    }
  }

  const [conversationHotkey] = conversationHotkeys;
  const exampleName = projects[0]?.spokenNames[0] ?? "my project";

  return (
    <PanelGrid>
      <Panel
        title="Projects"
        wide
        description={
          <>
            The folders you can name in a voice command, and how you say each one. Separate several
            spoken names with commas. Paths containing ; % or &quot; cannot be used: Windows
            Terminal would read those characters as commands or variables.
          </>
        }
      >
        <form
          className="inline-form inline-form--project"
          aria-label="Add project"
          onSubmit={(event) => {
            event.preventDefault();
            if (canAdd) {
              void addProject();
            }
          }}
        >
          <div className="field">
            <label className="field-label" htmlFor={folderId}>
              Folder
            </label>
            <input
              id={folderId}
              value={draftPath}
              onChange={(event) => setDraftPath(event.currentTarget.value)}
              placeholder="C:\Users\you\projects\app"
              spellCheck={false}
              aria-invalid={cleanedPath && folderProblem ? true : undefined}
              aria-describedby={cleanedPath && folderProblem ? problemId : undefined}
            />
          </div>
          <div className="field">
            <label className="field-label" htmlFor={namesId}>
              Spoken names
            </label>
            <input
              id={namesId}
              value={namesValue}
              onChange={(event) => {
                setDraftNames(event.currentTarget.value);
                setNamesEdited(true);
              }}
              placeholder="local flow, the app"
            />
          </div>
          <button type="submit" disabled={!canAdd}>
            <Plus size={16} aria-hidden="true" />
            {isSaving ? "Adding" : "Add"}
          </button>
        </form>
        {cleanedPath && folderProblem ? (
          <p className="field-hint field-hint--warning" id={problemId} role="status">
            {folderProblem}
          </p>
        ) : null}
        {error ? (
          <Notice tone="error" onDismiss={() => setError("")}>
            {error}
          </Notice>
        ) : null}

        {projects.length === 0 ? (
          <div className="empty-state">
            <p>No projects yet.</p>
            <p>
              Type a project folder&apos;s full path above and choose Add. Then start a conversation
              with {conversationHotkey} and say &ldquo;Open Claude Code in {exampleName}&rdquo;.
              Voice commands work only in a conversation, not in dictation.
            </p>
          </div>
        ) : (
          <ul className="setting-list" aria-label="Projects">
            {projects.map((project) => (
              <CodingProjectRow
                key={`${project.id}:${project.spokenNames.join("|")}`}
                project={project}
                onChanged={setProjects}
                onError={setError}
              />
            ))}
          </ul>
        )}
      </Panel>

      <Panel
        title="Agents found"
        actions={
          <button type="button" onClick={() => void checkAgain()} disabled={detecting}>
            <RefreshCw size={16} aria-hidden="true" />
            {detecting ? "Checking" : "Check again"}
          </button>
        }
      >
        {agents ? (
          <ul className="setting-list" aria-label="Agents found">
            {agents.agents.map((status) => {
              const view = describeAgent(status);
              return (
                <li className="agent-row" key={status.agent}>
                  <div className="agent-row__heading">
                    <strong>{view.name}</strong>
                    <StatusPill tone={view.tone}>{view.state}</StatusPill>
                  </div>
                  {view.location ? <span className="agent-row__path">{view.location}</span> : null}
                  <span className="agent-row__detail">{view.detail}</span>
                </li>
              );
            })}
            <li className="agent-row">
              <div className="agent-row__heading">
                <strong>Windows Terminal</strong>
                <StatusPill tone={agents.windowsTerminal ? "ok" : "blocked"}>
                  {agents.windowsTerminal ? "Found" : "Not found"}
                </StatusPill>
              </div>
              {agents.windowsTerminal ? (
                <span className="agent-row__path">{agents.windowsTerminal}</span>
              ) : (
                <span className="agent-row__detail">Needed to open Claude Code in a tab.</span>
              )}
            </li>
          </ul>
        ) : (
          <p className="empty-state" role="status">
            {detecting
              ? "Looking for coding agents..."
              : isDesktopApp()
                ? "No result yet. Choose Check again."
                : "Looking for coding agents needs the desktop app."}
          </p>
        )}
      </Panel>

      <Panel title="Saying a command">
        <p className="field-hint">
          Start a conversation first ({conversationHotkey}); commands are not recognised in
          dictation. Then say, for example:
        </p>
        <ul className="phrase-list">
          {examplePhrases(projects).map((phrase) => (
            <li key={phrase}>&ldquo;{phrase}&rdquo;</li>
          ))}
        </ul>
        <p className="field-hint">
          Every command is read back and runs only after you say yes. LocalFlow never types a task
          for you: once Claude Code opens, dictate the task into its prompt and press Enter. A
          question runs read-only in the background and the answer is spoken, starting &ldquo;Claude
          says&rdquo;. Say &ldquo;stop that&rdquo; to cancel it.
        </p>
        <p className="field-hint">
          A question you confirm goes to Anthropic through your own Claude subscription. LocalFlow
          itself makes no network connection, and its logs name only the agent and the project.
        </p>
      </Panel>
    </PanelGrid>
  );
}

function CodingProjectRow({
  project,
  onChanged,
  onError,
}: {
  project: CodingProjectView;
  onChanged: (projects: CodingProjectView[]) => void;
  onError: (message: string) => void;
}) {
  const [names, setNames] = useState(formatSpokenNames(project.spokenNames));

  async function commitNames() {
    const next = parseSpokenNames(names);
    if (next.join("|") === project.spokenNames.join("|")) {
      return;
    }
    try {
      onChanged(await saveCodingProject({ id: project.id, path: project.path, spokenNames: next }));
      onError("");
    } catch (reason) {
      onError(errorMessage(reason, "The spoken names could not be saved."));
      setNames(formatSpokenNames(project.spokenNames));
    }
  }

  async function remove() {
    try {
      onChanged(await removeCodingProject(project.id));
    } catch (reason) {
      onError(errorMessage(reason, "The project could not be removed."));
    }
  }

  return (
    <li className="setting-row setting-row--project">
      <div className="project-folder">
        <strong>{project.folderName}</strong>
        <span>{project.path}</span>
      </div>
      <input
        aria-label={`Spoken names for ${project.folderName}`}
        value={names}
        onChange={(event) => setNames(event.currentTarget.value)}
        onBlur={() => void commitNames()}
        onKeyDown={(event) => {
          if (event.key === "Enter") {
            void commitNames();
          }
        }}
      />
      <RemoveButton label={`Remove "${project.folderName}"`} onClick={() => void remove()} />
      {project.folderProblem || project.sharedNames.length > 0 ? (
        <div className="setting-row__notes">
          {project.folderProblem ? (
            <span className="field-hint--warning">{project.folderProblem}</span>
          ) : null}
          {project.sharedNames.length > 0 ? (
            <span className="field-hint--warning">
              Another project is also called {project.sharedNames.join(", ")}. Saying that name will
              be asked about.
            </span>
          ) : null}
        </div>
      ) : null}
    </li>
  );
}
