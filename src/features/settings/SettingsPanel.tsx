import { useEffect, useId, useState } from "react";
import type { ReactNode, SyntheticEvent } from "react";

import type { Settings } from "../../ipc";
import { Button } from "../../ui/Button";
import { ErrorNotice } from "../../ui/ErrorNotice";
import { RecordingDevicesForm } from "./RecordingDevicesForm";
import { shortcutFromKeyPress } from "./shortcutKeys";
import { useSettings } from "./useSettings";
import type { SettingsController } from "./useSettings";

interface SettingsPanelProps {
  onDirtyChange: (dirty: boolean) => void;
  /** Shown below the settings, such as where data is kept. */
  children?: ReactNode;
}

export function SettingsPanel({ onDirtyChange, children }: SettingsPanelProps) {
  const headingId = useId();
  const controller = useSettings();
  const { view } = controller;
  const [shortcutDirty, setShortcutDirty] = useState(false);
  const [devicesDirty, setDevicesDirty] = useState(false);
  const dirty = shortcutDirty || devicesDirty;

  useEffect(() => {
    onDirtyChange(dirty);
  }, [dirty, onDirtyChange]);

  return (
    <section className="stack" aria-labelledby={headingId}>
      <h2 id={headingId}>Settings</h2>
      <RecordingDevicesForm onDirtyChange={setDevicesDirty} />
      {view.phase === "loading" && (
        <p className="progress" role="status">
          Loading your settings…
        </p>
      )}
      {view.phase === "failed" && (
        <ErrorNotice error={view.error}>
          <Button onClick={controller.reload}>Try again</Button>
        </ErrorNotice>
      )}
      {view.phase === "ready" && (
        <>
          <ShortcutForm
            // A saved shortcut replaces the draft with its canonical spelling.
            key={`${view.settings.sectionShortcut}\n${String(view.settings.sectionShortcutRevision)}`}
            controller={controller}
            settings={view.settings}
            busy={view.busy}
            onDirtyChange={setShortcutDirty}
          />
          {view.error !== null && (
            <ErrorNotice error={view.error}>
              {view.error.actions.includes("reload") && (
                <Button onClick={controller.reload}>Load the saved version</Button>
              )}
            </ErrorNotice>
          )}
          <p className="visually-hidden" role="status">
            {view.announcement}
          </p>
        </>
      )}
      {children}
    </section>
  );
}

interface ShortcutFormProps {
  controller: SettingsController;
  settings: Settings;
  busy: boolean;
  onDirtyChange: (dirty: boolean) => void;
}

function ShortcutForm({ controller, settings, busy, onDirtyChange }: ShortcutFormProps) {
  const headingId = useId();
  const inputId = useId();
  const hintId = useId();
  const [draft, setDraft] = useState(settings.sectionShortcut);
  const dirty = draft !== settings.sectionShortcut;
  const isDefault = settings.sectionShortcut === settings.defaultSectionShortcut;

  const change = (value: string) => {
    setDraft(value);
    onDirtyChange(value !== settings.sectionShortcut);
  };

  const submit = (event: SyntheticEvent) => {
    event.preventDefault();
    if (busy || !dirty) {
      return;
    }
    void controller.setSectionShortcut(draft).then((saved) => {
      if (saved) {
        onDirtyChange(false);
      }
    });
  };

  return (
    <form className="panel form" onSubmit={submit} aria-labelledby={headingId}>
      <h3 id={headingId}>Next-section shortcut</h3>
      <p>
        While a meeting is being recorded, this shortcut will move on to the next agenda item,
        even when another app has focus.
      </p>
      <p className="notice">
        This build saves the shortcut but does not act on it yet: marking sections during a
        recording is not available.
      </p>
      <div className="field">
        <label htmlFor={inputId}>Shortcut</label>
        <input
          id={inputId}
          type="text"
          value={draft}
          maxLength={64}
          autoComplete="off"
          spellCheck={false}
          aria-describedby={hintId}
          onChange={(event) => {
            change(event.target.value);
          }}
          onKeyDown={(event) => {
            const pressed = shortcutFromKeyPress(event);
            if (pressed !== null) {
              event.preventDefault();
              change(pressed);
            }
          }}
        />
        <p id={hintId} className="hint">
          Press the combination while this field has focus, or type it, for example
          Ctrl+Shift+F9. Use Ctrl, Alt or the Windows key, optionally Shift, with a letter, a
          digit or F1 to F24.
        </p>
      </div>
      <div className="form__actions">
        <Button type="submit" variant="primary" unavailable={busy || !dirty}>
          Save shortcut
        </Button>
        <Button
          unavailable={busy || (isDefault && !dirty)}
          onClick={() => {
            change(settings.defaultSectionShortcut);
          }}
        >
          Use the default ({settings.defaultSectionShortcut})
        </Button>
        {dirty && <span className="hint">Unsaved changes</span>}
      </div>
    </form>
  );
}
