import { useEffect, useRef, useState } from "react";

import { call } from "../../ipc";
import type { CommandError, CommandResult, DeviceChoiceArg, RecordingDevices } from "../../ipc";

export type DevicesView =
  | { phase: "loading" }
  | { phase: "failed"; error: CommandError }
  | {
      phase: "ready";
      devices: RecordingDevices;
      busy: boolean;
      /** Why the last change was not saved. */
      error: CommandError | null;
      /** What the last action did, for a screen reader. */
      announcement: string;
    };

export interface DevicesController {
  view: DevicesView;
  /** Reads the saved choices and the connected devices again. */
  reload: () => void;
  /** Resolves to whether the choices were saved. */
  save: (microphone: DeviceChoiceArg, system: DeviceChoiceArg) => Promise<boolean>;
}

function shown(result: CommandResult<RecordingDevices>, announcement: string): DevicesView {
  return result.ok
    ? { phase: "ready", devices: result.value, busy: false, error: null, announcement }
    : { phase: "failed", error: result.error };
}

/** Which devices recordings are made from. Rust lists the devices and validates the choice. */
export function useRecordingDevices(): DevicesController {
  const [view, setView] = useState<DevicesView>({ phase: "loading" });
  const inFlight = useRef(false);

  useEffect(() => {
    let current = true;
    void call("recording_devices").then((result) => {
      if (current) {
        setView(shown(result, ""));
      }
    });
    return () => {
      current = false;
    };
  }, []);

  return {
    view,
    reload: () => {
      if (inFlight.current) {
        return;
      }
      inFlight.current = true;
      setView((current) =>
        current.phase === "ready" ? { ...current, busy: true, announcement: "" } : current,
      );
      void call("recording_devices").then((result) => {
        inFlight.current = false;
        setView(shown(result, "Device list refreshed."));
      });
    },
    save: async (microphone, system) => {
      if (view.phase !== "ready" || inFlight.current) {
        return false;
      }
      inFlight.current = true;
      setView({ ...view, busy: true, error: null, announcement: "" });
      const result = await call("recording_set_devices", {
        microphone,
        system,
        expectedRevision: view.devices.revision,
      });
      inFlight.current = false;
      setView((current) => {
        if (current.phase !== "ready") {
          return current;
        }
        return result.ok
          ? shown(result, "Recording devices saved.")
          : { ...current, busy: false, error: result.error };
      });
      return result.ok;
    },
  };
}
