import { useEffect, useRef, useState } from "react";

import { call } from "../../ipc";
import type { CommandError, Settings } from "../../ipc";

export type SettingsView =
  | { phase: "loading" }
  | { phase: "failed"; error: CommandError }
  | {
      phase: "ready";
      settings: Settings;
      busy: boolean;
      /** Why the last change was not saved. */
      error: CommandError | null;
      /** What the last change did, for a screen reader. */
      announcement: string;
    };

export interface SettingsController {
  view: SettingsView;
  reload: () => void;
  /** Resolves to whether the shortcut was saved. */
  setSectionShortcut: (shortcut: string) => Promise<boolean>;
}

export function useSettings(): SettingsController {
  const [view, setView] = useState<SettingsView>({ phase: "loading" });
  const inFlight = useRef(false);

  const load = (isCurrent: () => boolean) => {
    void call("settings_get").then((result) => {
      if (isCurrent()) {
        setView(
          result.ok
            ? { phase: "ready", settings: result.value, busy: false, error: null, announcement: "" }
            : { phase: "failed", error: result.error },
        );
      }
    });
  };

  useEffect(() => {
    let current = true;
    void call("settings_get").then((result) => {
      if (current) {
        setView(
          result.ok
            ? { phase: "ready", settings: result.value, busy: false, error: null, announcement: "" }
            : { phase: "failed", error: result.error },
        );
      }
    });
    return () => {
      current = false;
    };
  }, []);

  return {
    view,
    reload: () => {
      load(() => true);
    },
    setSectionShortcut: async (shortcut) => {
      if (view.phase !== "ready" || inFlight.current) {
        return false;
      }
      inFlight.current = true;
      setView({ ...view, busy: true, error: null, announcement: "" });
      const result = await call("settings_set_section_shortcut", {
        shortcut,
        expectedRevision: view.settings.sectionShortcutRevision,
      });
      inFlight.current = false;
      setView((current) => {
        if (current.phase !== "ready") {
          return current;
        }
        return result.ok
          ? {
              phase: "ready",
              settings: result.value,
              busy: false,
              error: null,
              announcement: `Shortcut saved as ${result.value.sectionShortcut}.`,
            }
          : { ...current, busy: false, error: result.error };
      });
      return result.ok;
    },
  };
}
