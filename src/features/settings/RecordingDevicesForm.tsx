import { useEffect, useId, useState } from "react";
import type { SyntheticEvent } from "react";

import type { DeviceChoice, DeviceChoiceArg, RecordingDevices, SourceDevices } from "../../ipc";
import { Button } from "../../ui/Button";
import { ErrorNotice } from "../../ui/ErrorNotice";
import { useRecordingDevices } from "./useRecordingDevices";
import type { DevicesController } from "./useRecordingDevices";

const DEVICE_PREFIX = "device:";

/** A choice as the value of an option in the list. */
function toValue(choice: DeviceChoice): string {
  return choice.kind === "device" ? `${DEVICE_PREFIX}${choice.id}` : choice.kind;
}

function toArg(value: string): DeviceChoiceArg {
  if (value.startsWith(DEVICE_PREFIX)) {
    return { kind: "device", deviceId: value.slice(DEVICE_PREFIX.length) };
  }
  return value === "communications" ? { kind: "communications" } : { kind: "default" };
}

/** Which microphone and which playback device recordings are made from. */
export function RecordingDevicesForm({ onDirtyChange }: { onDirtyChange: (dirty: boolean) => void }) {
  const headingId = useId();
  const controller = useRecordingDevices();
  const { view } = controller;

  return (
    <section className="panel form" aria-labelledby={headingId}>
      <h3 id={headingId}>Recording devices</h3>
      {view.phase === "loading" && (
        <p className="progress" role="status">
          Looking for your devices…
        </p>
      )}
      {view.phase === "failed" && (
        <ErrorNotice error={view.error}>
          <Button onClick={controller.reload}>Try again</Button>
        </ErrorNotice>
      )}
      {view.phase === "ready" && (
        <>
          <DeviceChoices
            // Saved choices replace the draft.
            key={`${String(view.devices.revision)}\n${toValue(view.devices.microphone.choice)}\n${toValue(view.devices.system.choice)}`}
            controller={controller}
            devices={view.devices}
            busy={view.busy}
            onDirtyChange={onDirtyChange}
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
    </section>
  );
}

interface DeviceChoicesProps {
  controller: DevicesController;
  devices: RecordingDevices;
  busy: boolean;
  onDirtyChange: (dirty: boolean) => void;
}

function DeviceChoices({ controller, devices, busy, onDirtyChange }: DeviceChoicesProps) {
  const savedMicrophone = toValue(devices.microphone.choice);
  const savedSystem = toValue(devices.system.choice);
  const [microphone, setMicrophone] = useState(savedMicrophone);
  const [system, setSystem] = useState(savedSystem);
  const dirty = microphone !== savedMicrophone || system !== savedSystem;

  useEffect(() => {
    onDirtyChange(dirty);
    return () => {
      onDirtyChange(false);
    };
  }, [dirty, onDirtyChange]);

  const submit = (event: SyntheticEvent) => {
    event.preventDefault();
    if (busy || !dirty) {
      return;
    }
    void controller.save(toArg(microphone), toArg(system));
  };

  return (
    <form className="form stack" onSubmit={submit} aria-label="Recording devices">
      <p>
        Skribe records two separate tracks: a microphone, and everything one playback device
        plays. Choose which devices those are. A change applies from the next recording.
      </p>
      <p>
        Meeting apps often play through Windows&rsquo; default communications device, which can
        be a different device from the default one. If the other people in a meeting are
        missing from a recording, choose the communications device, or the exact device, for
        system audio.
      </p>
      {devices.synthetic && (
        <p className="notice notice--error">
          This is a development build. The devices listed are generated test tones, not your
          real devices.
        </p>
      )}
      <DeviceSelect
        label="Microphone"
        source={devices.microphone}
        value={microphone}
        onChange={setMicrophone}
      />
      <DeviceSelect
        label="System audio (playback device)"
        source={devices.system}
        value={system}
        onChange={setSystem}
      />
      <div className="form__actions">
        <Button type="submit" variant="primary" unavailable={busy || !dirty}>
          Save devices
        </Button>
        <Button unavailable={busy} onClick={controller.reload}>
          Refresh the list
        </Button>
        {dirty && <span className="hint">Unsaved changes</span>}
      </div>
    </form>
  );
}

interface DeviceSelectProps {
  label: string;
  source: SourceDevices;
  value: string;
  onChange: (value: string) => void;
}

function DeviceSelect({ label, source, value, onChange }: DeviceSelectProps) {
  const selectId = useId();
  const noteId = useId();
  const { choice, available } = source;
  const current = (wanted: "isDefault" | "isCommunicationsDefault") => {
    const device = available.find((candidate) => candidate[wanted]);
    return device === undefined ? "none connected" : `currently ${device.name}`;
  };
  // A chosen device that is unplugged stays in the list, so the choice is not lost.
  const missing =
    choice.kind === "device" && !available.some((device) => device.id === choice.id)
      ? choice
      : null;

  return (
    <div className="field">
      <label htmlFor={selectId}>{label}</label>
      <select
        id={selectId}
        value={value}
        aria-describedby={noteId}
        onChange={(event) => {
          onChange(event.target.value);
        }}
      >
        <option value="default">Windows default device ({current("isDefault")})</option>
        <option value="communications">
          Windows default communications device ({current("isCommunicationsDefault")})
        </option>
        {available.map((device) => (
          <option key={device.id} value={`${DEVICE_PREFIX}${device.id}`}>
            {device.name}
          </option>
        ))}
        {missing !== null && (
          <option value={`${DEVICE_PREFIX}${missing.id}`}>{missing.name} (not connected)</option>
        )}
      </select>
      <div id={noteId}>
        {source.unavailable !== null ? (
          <p className="field__problem">
            {source.unavailable.message} {source.unavailable.recovery}
          </p>
        ) : (
          !source.choiceAvailable && (
            <p className="field__problem">
              The saved choice is not connected. Recording will not start until it is connected
              again or another device is chosen. Skribe does not switch devices on its own.
            </p>
          )
        )}
      </div>
    </div>
  );
}
