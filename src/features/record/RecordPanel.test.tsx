import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, test, vi } from "vitest";

import { call, listen } from "../../ipc";
import type { MeetingDetail, PreflightReport } from "../../ipc";
import {
  conflict,
  detail,
  failed,
  idle,
  liveTrack,
  noMicrophone,
  ok,
  recordingStatus,
  trackSummary,
} from "../../test/fixtures";
import { answer, captureEvents } from "../../test/ipc";
import { formatDuration, formatRoom } from "./duration";
import { RecordPanel } from "./RecordPanel";

vi.mock("../../ipc", () => ({ call: vi.fn(), listen: vi.fn() }));
const callMock = vi.mocked(call);
const listenMock = vi.mocked(listen);

beforeEach(() => {
  callMock.mockReset();
  listenMock.mockReset();
});

const meetingId = detail().meeting.id;
const recording = recordingStatus(meetingId);

const report: PreflightReport = {
  captureName: "Windows audio (WASAPI, shared mode)",
  synthetic: false,
  devices: [
    {
      source: "microphone",
      name: "Headset Microphone",
      format: "48000 Hz, 1 channel, 32-bit float",
      choice: { kind: "device", id: "mic-headset", name: "Headset Microphone" },
    },
    {
      source: "system",
      name: "Speakers",
      format: "48000 Hz, 2 channels, 32-bit float",
      choice: { kind: "communications" },
    },
  ],
  availableSeconds: 130_000,
  minimumSeconds: 3_600,
};

function show(loaded: MeetingDetail = detail(["Intro"])) {
  const events = captureEvents(listenMock);
  const props = { onMeetingStale: vi.fn(), onMeeting: vi.fn() };
  render(<RecordPanel detail={loaded} {...props} />);
  return { events, ...props };
}

test("before recording it explains the two tracks and that there is no pause", async () => {
  answer(callMock, { recording_status: ok(idle) });
  show();

  expect(screen.getByRole("heading", { name: "Recording" })).toBeInTheDocument();
  expect(screen.getByText(/two separate tracks/)).toBeInTheDocument();
  expect(screen.getByText(/unless you choose devices in Settings/)).toBeInTheDocument();
  expect(screen.getByText(/cannot be paused or resumed/)).toBeInTheDocument();
  await waitFor(() => {
    expect(screen.getByRole("button", { name: "Start recording" })).toHaveAttribute(
      "aria-disabled",
      "false",
    );
  });
  expect(screen.queryByRole("button", { name: /pause/i })).not.toBeInTheDocument();
});

test("checking devices shows what would be recorded and the room on disk", async () => {
  const user = userEvent.setup();
  answer(callMock, { recording_status: ok(idle), recording_preflight: ok(report) });
  show();

  await user.click(screen.getByRole("button", { name: "Check devices" }));

  const devices = await screen.findByRole("region", { name: "Devices that will be recorded" });
  expect(devices).toHaveTextContent("Headset Microphone (48000 Hz, 1 channel, 32-bit float)");
  expect(devices).toHaveTextContent("Chosen in Settings");
  expect(devices).toHaveTextContent("Speakers");
  expect(devices).toHaveTextContent("The Windows default communications device");
  expect(devices).toHaveTextContent("about 36 hours of recording");
  expect(devices).toHaveTextContent("at least 60 minutes to start");
  expect(callMock).toHaveBeenCalledWith("recording_preflight", { meetingId });
  expect(screen.queryByText(/generated test tone/)).not.toBeInTheDocument();
});

test("a development build says plainly that it records a test tone", async () => {
  const user = userEvent.setup();
  answer(callMock, {
    recording_status: ok(idle),
    recording_preflight: ok({ ...report, synthetic: true }),
  });
  show();

  await user.click(screen.getByRole("button", { name: "Check devices" }));

  expect(await screen.findByText(/It is not recording your microphone/)).toBeInTheDocument();
});

test("a refused start says which device and why, and that nothing was recorded", async () => {
  const user = userEvent.setup();
  answer(callMock, { recording_status: ok(idle), recording_start: failed(noMicrophone) });
  const { onMeetingStale } = show();

  await user.click(screen.getByRole("button", { name: "Start recording" }));

  const alert = await screen.findByRole("alert");
  expect(alert).toHaveTextContent("Windows has no default microphone.");
  expect(alert).toHaveTextContent("Nothing was recorded");
  expect(screen.getByRole("button", { name: "Start recording" })).toBeInTheDocument();
  expect(onMeetingStale).toHaveBeenCalledTimes(1);
});

test("starting shows both tracks live and reloads the meeting", async () => {
  const user = userEvent.setup();
  answer(callMock, { recording_status: ok(idle), recording_start: ok(recording) });
  const { onMeetingStale } = show();

  await user.click(screen.getByRole("button", { name: "Start recording" }));

  expect(callMock).toHaveBeenCalledWith("recording_start", { meetingId });
  const tracks = await screen.findByRole("list", { name: "Tracks being recorded" });
  const rows = within(tracks).getAllByRole("listitem");
  expect(rows).toHaveLength(2);
  expect(rows[0]).toHaveTextContent("Microphone");
  expect(rows[0]).toHaveTextContent("Headset Microphone · 48000 Hz, 1 channel, 32-bit float");
  expect(rows[0]).toHaveTextContent("Capturing · 1:02 of audio");
  expect(rows[1]).toHaveTextContent("System audio");
  expect(screen.getByRole("meter", { name: "Microphone level" })).toHaveValue(0.4);
  expect(screen.getByText("1:02")).toBeInTheDocument();
  expect(onMeetingStale).toHaveBeenCalledTimes(1);
  expect(screen.queryByRole("button", { name: "Start recording" })).not.toBeInTheDocument();
});

test("pushed status updates the levels without erasing an error on screen", async () => {
  const user = userEvent.setup();
  answer(callMock, {
    recording_status: ok(recording),
    recording_stop: failed(conflict),
  });
  const { events } = show(detail(["Intro"], { recordingState: "recording" }));
  await screen.findByRole("list", { name: "Tracks being recorded" });

  await user.click(screen.getByRole("button", { name: "Stop recording…" }));
  await user.click(screen.getByRole("button", { name: "Stop and save" }));
  expect(await screen.findByRole("alert")).toHaveTextContent(conflict.message);

  act(() => {
    events.emit("recording-status", {
      phase: "recording",
      meetingId,
      synthetic: false,
      elapsedUs: 125_000_000,
      tracks: [
        liveTrack({ level: 0.9, capturedUs: 124_000_000 }),
        liveTrack({ source: "system", deviceName: "Speakers" }),
      ],
    });
  });

  expect(screen.getByRole("meter", { name: "Microphone level" })).toHaveValue(0.9);
  expect(screen.getByText("2:05")).toBeInTheDocument();
  // The failure the user has not yet acted on is still there.
  expect(screen.getByRole("alert")).toHaveTextContent(conflict.message);
});

test("a lost device is announced; the other track is shown still capturing", async () => {
  answer(callMock, { recording_status: ok(recording) });
  const { events } = show(detail([], { recordingState: "recording" }));
  await screen.findByRole("list", { name: "Tracks being recorded" });

  act(() => {
    events.emit(
      "recording-status",
      recordingStatus(meetingId, [
        liveTrack(),
        liveTrack({
          source: "system",
          deviceName: "Speakers",
          capturing: false,
          problem: "The device was disconnected or disabled during the recording.",
          stoppedAtUs: 30_000_000,
        }),
      ]),
    );
  });

  const alert = screen.getByRole("alert");
  expect(alert).toHaveTextContent("System audio stopped.");
  expect(alert).toHaveTextContent("disconnected or disabled");
  expect(alert).toHaveTextContent("no other device was switched in");
  expect(screen.getByRole("meter", { name: "Microphone level" })).toBeInTheDocument();
  expect(screen.queryByRole("meter", { name: "System audio level" })).not.toBeInTheDocument();
});

test("dropped frames are reported while recording", async () => {
  answer(callMock, {
    recording_status: ok(recordingStatus(meetingId, [liveTrack({ droppedFrames: 960 })])),
  });
  show(detail([], { recordingState: "recording" }));

  expect(await screen.findByText(/960 audio frames were dropped/)).toBeInTheDocument();
});

test("stopping asks first, says it cannot be resumed, and hands back the saved meeting", async () => {
  const user = userEvent.setup();
  const recorded = detail(
    ["Intro"],
    { recordingState: "recorded", recordingDurationUs: 3_725_000_000 },
    { agendaEditable: false, tracks: [trackSummary()] },
  );
  answer(callMock, { recording_status: ok(recording), recording_stop: ok(recorded) });
  const { onMeeting } = show(detail(["Intro"], { recordingState: "recording" }));

  await user.click(await screen.findByRole("button", { name: "Stop recording…" }));
  const question = screen.getByRole("group", { name: "Stop recording?" });
  expect(question).toHaveTextContent("cannot be resumed");
  expect(screen.getByRole("button", { name: "Cancel" })).toHaveFocus();
  expect(callMock).not.toHaveBeenCalledWith("recording_stop", expect.anything());

  await user.click(within(question).getByRole("button", { name: "Stop and save" }));

  expect(callMock).toHaveBeenCalledWith("recording_stop", { meetingId });
  await waitFor(() => {
    expect(onMeeting).toHaveBeenCalledExactlyOnceWith(recorded);
  });
});

test("while the tracks are being completed it says to keep Skribe open", async () => {
  answer(callMock, { recording_status: ok({ phase: "finalising", meetingId }) });
  show(detail([], { recordingState: "finalising" }));

  expect(await screen.findByText(/completing both track files/)).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: /Start|Stop/ })).not.toBeInTheDocument();
});

test("a finished recording reports each track exactly as it ended", async () => {
  answer(callMock, { recording_status: ok(idle) });
  show(
    detail(
      [],
      { recordingState: "recorded", recordingDurationUs: 3_725_000_000 },
      {
        agendaEditable: false,
        tracks: [
          trackSummary(),
          trackSummary({
            id: "t-system",
            source: "system",
            deviceName: "Speakers",
            health: "failed",
            problem: "The device was disconnected or disabled during the recording.",
            audioUs: 1_800_000_000,
            sizeBytes: 345_600_000,
          }),
        ],
      },
    ),
  );

  expect(screen.getByText(/1:02:05 long/)).toBeInTheDocument();
  expect(screen.getByText(/are not available in this build/)).toBeInTheDocument();
  const rows = within(screen.getByRole("list", { name: "Recorded tracks" })).getAllByRole(
    "listitem",
  );
  expect(rows[0]).toHaveTextContent("Microphone: Complete");
  expect(rows[0]).toHaveTextContent("1:02:03 of audio · 682 MB");
  expect(rows[0]).toHaveTextContent("ab".repeat(32));
  expect(rows[1]).toHaveTextContent("System audio: Failed");
  expect(rows[1]).toHaveTextContent("disconnected or disabled");
  expect(rows[1]).toHaveTextContent("30:00 of audio");
  await waitFor(() => {
    expect(callMock).toHaveBeenCalledWith("recording_status");
  });
});

test("a system track with nothing played is described as that, not as a failure", () => {
  answer(callMock, { recording_status: ok(idle) });
  show(
    detail(
      [],
      { recordingState: "recorded" },
      { tracks: [trackSummary({ source: "system", audioUs: 0, sizeBytes: 0, startUs: null })] },
    ),
  );

  expect(screen.getByText(/nothing was played during the recording/)).toBeInTheDocument();
  expect(screen.getByText("System audio: Complete")).toBeInTheDocument();
});

test("a failed recording says so and still lists the track that is intact", () => {
  answer(callMock, { recording_status: ok(idle) });
  show(
    detail(
      [],
      { recordingState: "failed" },
      {
        tracks: [
          trackSummary(),
          trackSummary({
            id: "t-system",
            source: "system",
            finalisation: "failed",
            health: "failed",
            problem: "The track's file could not be completed.",
            audioUs: null,
            sizeBytes: null,
            sha256: null,
          }),
        ],
      },
    ),
  );

  expect(screen.getByText(/The recording failed/)).toBeInTheDocument();
  expect(screen.getByText("Microphone: Complete")).toBeInTheDocument();
  expect(screen.getByText("System audio: Failed")).toBeInTheDocument();
  expect(screen.getByText(/No valid file/)).toBeInTheDocument();
});

test("an interrupted recording is explained and recovered on request", async () => {
  const user = userEvent.setup();
  const pending = [
    trackSummary({ finalisation: "pending", audioUs: null, sizeBytes: null, sha256: null }),
  ];
  const recovered = detail(
    [],
    { recordingState: "recorded" },
    {
      tracks: [
        trackSummary({
          finalisation: "recovered",
          health: "degraded",
          problem: "The recording was interrupted.",
        }),
      ],
    },
  );
  answer(callMock, { recording_status: ok(idle), recording_recover: ok(recovered) });
  const { onMeeting } = show(detail([], { recordingState: "interrupted" }, { tracks: pending }));

  expect(screen.getByText(/still running when Skribe closed/)).toBeInTheDocument();
  expect(screen.getByText("Microphone: not completed")).toBeInTheDocument();

  await user.click(screen.getByRole("button", { name: "Recover the recording" }));

  expect(callMock).toHaveBeenCalledWith("recording_recover", { meetingId });
  await waitFor(() => {
    expect(onMeeting).toHaveBeenCalledExactlyOnceWith(recovered);
  });
});

test("a recovered track is labelled as recovered", () => {
  answer(callMock, { recording_status: ok(idle) });
  show(
    detail(
      [],
      { recordingState: "recorded" },
      {
        tracks: [
          trackSummary({
            finalisation: "recovered",
            health: "degraded",
            problem: "The recording was interrupted.",
          }),
        ],
      },
    ),
  );

  expect(screen.getByText("Microphone: Recovered after an interruption")).toBeInTheDocument();
  expect(screen.getByText("The recording was interrupted.")).toBeInTheDocument();
});

test("a meeting cannot be started while another is being recorded", async () => {
  answer(callMock, { recording_status: ok(recordingStatus("another-meeting")) });
  show();

  expect(await screen.findByText(/Another meeting is being recorded/)).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Start recording" })).not.toBeInTheDocument();
});

test("stops listening when it leaves the screen", () => {
  answer(callMock, { recording_status: ok(idle) });
  const events = captureEvents(listenMock);
  const { unmount } = render(
    <RecordPanel detail={detail()} onMeetingStale={vi.fn()} onMeeting={vi.fn()} />,
  );

  unmount();

  expect(events.stopped).toEqual(["recording-status"]);
});

test("durations and disk room are written for people", () => {
  expect(formatDuration(0)).toBe("0:00");
  expect(formatDuration(59_999_999)).toBe("0:59");
  expect(formatDuration(3_725_000_000)).toBe("1:02:05");
  expect(formatRoom(130_000)).toBe("about 36 hours");
  expect(formatRoom(3_600)).toBe("about 60 minutes");
  expect(formatRoom(60)).toBe("about 1 minute");
});
