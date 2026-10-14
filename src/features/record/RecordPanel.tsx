import { useId } from "react";

import type {
  DeviceChoice,
  LiveTrack,
  MeetingDetail,
  PreflightReport,
  TrackHealth,
  TrackSource,
  TrackSummary,
} from "../../ipc";
import { Button } from "../../ui/Button";
import { ConfirmPanel, ConfirmTrigger, useConfirmation } from "../../ui/Confirmation";
import { ErrorNotice } from "../../ui/ErrorNotice";
import { formatBytes } from "../../ui/format";
import { formatDuration, formatRoom } from "./duration";
import { useRecording } from "./useRecording";
import type { RecordingAction, RecordingController } from "./useRecording";

interface RecordPanelProps {
  detail: MeetingDetail;
  /** Loads the meeting's saved form again. */
  onMeetingStale: () => void;
  /** Rust returned the meeting's new form. */
  onMeeting: (detail: MeetingDetail) => void;
}

const sourceLabel: Record<TrackSource, string> = {
  microphone: "Microphone",
  system: "System audio",
};

const choiceLabel: Record<DeviceChoice["kind"], string> = {
  default: "The Windows default device",
  communications: "The Windows default communications device",
  device: "Chosen in Settings",
};

const healthLabel: Record<TrackHealth, string> = {
  healthy: "Complete",
  degraded: "Complete, with audio missing",
  failed: "Failed",
};

const actionText: Record<Exclude<RecordingAction, null>, string> = {
  checking: "Checking your devices…",
  starting: "Checking your devices and starting…",
  stopping: "Saving the recording: completing and checking both tracks…",
  recovering: "Rebuilding the tracks from what was saved…",
};

/** Recording for one meeting: start, live track health, stop, results and recovery. */
export function RecordPanel({ detail, onMeetingStale, onMeeting }: RecordPanelProps) {
  const headingId = useId();
  const { meeting } = detail;
  const controller = useRecording(meeting.id, { onMeetingStale, onMeeting });
  const { view } = controller;
  const { status } = view;
  const liveElsewhere =
    status !== null && status.phase !== "idle" && status.meetingId !== meeting.id;
  const liveHere = status !== null && status.phase !== "idle" && status.meetingId === meeting.id;

  return (
    <section className="panel" aria-labelledby={headingId}>
      <h3 id={headingId}>Recording</h3>
      {view.error !== null && (
        <ErrorNotice error={view.error}>
          {view.error.actions.includes("reload") && (
            <Button onClick={onMeetingStale}>Load the saved version</Button>
          )}
        </ErrorNotice>
      )}
      <p className="progress" role="status">
        {view.action === null ? "" : actionText[view.action]}
      </p>
      {meeting.trashedAtUnixMs !== null ? (
        <TrackResults detail={detail} />
      ) : liveHere && status.phase === "recording" ? (
        <Live
          elapsedUs={status.elapsedUs}
          synthetic={status.synthetic}
          tracks={status.tracks}
          busy={view.action !== null}
          onStop={controller.stop}
        />
      ) : liveHere && status.phase === "starting" ? (
        <p>Starting the recording…</p>
      ) : liveHere ? (
        <p>
          Capture has stopped. Skribe is completing both track files and checking them. Keep
          Skribe open until this finishes.
        </p>
      ) : meeting.recordingState === "setup" ? (
        <Start controller={controller} blocked={liveElsewhere} loading={status === null} />
      ) : meeting.recordingState === "interrupted" ? (
        <Interrupted controller={controller} detail={detail} />
      ) : meeting.recordingState === "recorded" || meeting.recordingState === "failed" ? (
        <TrackResults detail={detail} />
      ) : (
        status !== null && (
          // The library says this meeting is being recorded, but nothing is capturing it.
          <p className="notice">
            This meeting&rsquo;s recording is not running in this window. Restart Skribe: the
            recording will be marked as interrupted and can then be recovered.
          </p>
        )
      )}
    </section>
  );
}

interface StartProps {
  controller: RecordingController;
  /** Another meeting is being recorded. */
  blocked: boolean;
  loading: boolean;
}

function Start({ controller, blocked, loading }: StartProps) {
  const { view } = controller;
  const busy = view.action !== null || loading;
  return (
    <>
      <p>
        Skribe records two separate tracks: a microphone, and what your computer plays through
        one playback device. Both are the Windows defaults unless you choose devices in
        Settings. The agenda cannot be changed once recording starts, and a recording cannot be
        paused or resumed.
      </p>
      {blocked ? (
        <p className="notice">
          Another meeting is being recorded. Stop that recording before starting this one.
        </p>
      ) : (
        <div className="form__actions">
          <Button variant="primary" unavailable={busy} onClick={controller.start}>
            Start recording
          </Button>
          <Button unavailable={busy} onClick={controller.check}>
            Check devices
          </Button>
        </div>
      )}
      {view.preflight !== null && <Preflight report={view.preflight} />}
    </>
  );
}

function SyntheticNotice() {
  return (
    <p className="notice notice--error">
      This is a development build recording a generated test tone. It is not recording your
      microphone or your computer&rsquo;s audio.
    </p>
  );
}

function Preflight({ report }: { report: PreflightReport }) {
  const headingId = useId();
  return (
    <section className="stack" aria-labelledby={headingId}>
      <h4 id={headingId}>Devices that will be recorded</h4>
      {report.synthetic && <SyntheticNotice />}
      <dl className="facts facts--compact">
        {report.devices.map((device) => (
          <div className="facts__row" key={device.source}>
            <dt>{sourceLabel[device.source]}</dt>
            <dd>
              {device.name} <span className="hint">({device.format})</span>
              <br />
              <span className="hint">{choiceLabel[device.choice.kind]}</span>
            </dd>
          </div>
        ))}
        <div className="facts__row">
          <dt>Room on disk</dt>
          <dd>
            {formatRoom(report.availableSeconds)} of recording. Skribe needs room for at least{" "}
            {formatRoom(report.minimumSeconds).replace("about ", "")} to start.
          </dd>
        </div>
      </dl>
    </section>
  );
}

interface LiveProps {
  elapsedUs: number;
  synthetic: boolean;
  tracks: LiveTrack[];
  busy: boolean;
  onStop: () => void;
}

function Live({ elapsedUs, synthetic, tracks, busy, onStop }: LiveProps) {
  const confirmation = useConfirmation();
  return (
    <>
      {synthetic && <SyntheticNotice />}
      <p className="lead">
        Recording <span className="clock">{formatDuration(elapsedUs)}</span>
      </p>
      <ul className="plain-list" aria-label="Tracks being recorded">
        {tracks.map((track) => (
          <LiveTrackRow key={track.source} track={track} />
        ))}
      </ul>
      <div>
        <ConfirmTrigger confirmation={confirmation} unavailable={busy}>
          Stop recording…
        </ConfirmTrigger>
      </div>
      <ConfirmPanel
        confirmation={confirmation}
        question="Stop recording?"
        confirmLabel="Stop and save"
        onConfirm={onStop}
      >
        <p>
          Both tracks are stopped together and saved. A stopped recording cannot be resumed.
        </p>
      </ConfirmPanel>
    </>
  );
}

function LiveTrackRow({ track }: { track: LiveTrack }) {
  const label = sourceLabel[track.source];
  return (
    <li className="row">
      <p className="row__title">{label}</p>
      <p className="row__meta">
        {track.deviceName} · {track.format}
      </p>
      {track.capturing ? (
        <p className="track__level">
          <meter
            className="meter"
            aria-label={`${label} level`}
            min={0}
            max={1}
            value={track.level}
          />
          <span>Capturing · {formatDuration(track.capturedUs)} of audio</span>
        </p>
      ) : (
        <p className="notice notice--error" role="alert">
          <strong>{label} stopped.</strong> {track.problem} The other track is still being
          recorded; no other device was switched in.
        </p>
      )}
      {track.droppedFrames > 0 && (
        <p className="field__problem">
          {String(track.droppedFrames)} audio frames were dropped because the computer could not
          keep up. The gaps are marked on the track.
        </p>
      )}
    </li>
  );
}

function Interrupted({ controller, detail }: { controller: RecordingController; detail: MeetingDetail }) {
  return (
    <>
      <p>
        This recording was still running when Skribe closed, so its tracks were never
        completed. Their audio is still on disk. Recovering rebuilds each track from what had
        been saved; at most the last second before the interruption is missing.
      </p>
      <div>
        <Button variant="primary" unavailable={controller.view.action !== null} onClick={controller.recover}>
          Recover the recording
        </Button>
      </div>
      <TrackList tracks={detail.tracks} />
    </>
  );
}

function TrackResults({ detail }: { detail: MeetingDetail }) {
  const { meeting, tracks } = detail;
  if (tracks.length === 0) {
    return <p className="empty-note">This meeting has not been recorded.</p>;
  }
  return (
    <>
      {meeting.recordingState === "failed" ? (
        <p className="notice notice--error">
          The recording failed: at least one track has no valid file. Tracks that are intact
          are kept and listed below.
        </p>
      ) : (
        meeting.recordingState === "recorded" && (
          <p>
            Recorded
            {meeting.recordingDurationUs === null
              ? ""
              : `, ${formatDuration(meeting.recordingDurationUs)} long`}
            . Both original tracks are kept as separate files in your library. Playing them
            back and transcribing them are not available in this build.
          </p>
        )
      )}
      <TrackList tracks={tracks} />
    </>
  );
}

function TrackList({ tracks }: { tracks: TrackSummary[] }) {
  return (
    <ul className="plain-list" aria-label="Recorded tracks">
      {tracks.map((track) => (
        <TrackRow key={track.id} track={track} />
      ))}
    </ul>
  );
}

function describeAudio(track: TrackSummary): string {
  if (track.audioUs === null || track.sizeBytes === null) {
    return "No valid file";
  }
  if (track.audioUs === 0) {
    return track.source === "system"
      ? "No audio: nothing was played during the recording"
      : "No audio";
  }
  return `${formatDuration(track.audioUs)} of audio · ${formatBytes(track.sizeBytes)}`;
}

function TrackRow({ track }: { track: TrackSummary }) {
  const pending = track.finalisation === "pending";
  return (
    <li className="row">
      <p className="row__title">
        {sourceLabel[track.source]}:{" "}
        {pending
          ? "not completed"
          : track.finalisation === "recovered"
            ? "Recovered after an interruption"
            : healthLabel[track.health]}
      </p>
      <p className="row__meta">
        {track.deviceName} · {track.format}
        {pending ? "" : ` · ${describeAudio(track)}`}
      </p>
      {track.problem !== null && <p>{track.problem}</p>}
      {track.sha256 !== null && (
        <details className="details">
          <summary>File checksum</summary>
          <p>
            SHA-256 <code className="wrap">{track.sha256}</code>
          </p>
        </details>
      )}
    </li>
  );
}
