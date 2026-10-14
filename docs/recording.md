# Recording

How Skribe records a meeting's audio, what it writes to disk, and how an interrupted recording
is recovered. The capture and file code is `crates/skribe-audio`, the only crate that touches
audio devices; the service that connects it to meetings is
`crates/skribe-ipc/src/recording.rs`.

**Status.** Everything here is implemented and tested in WSL against a synthetic backend, and
the Windows capture module type-checks for the Windows target. **It has not yet recorded from a
real device.** The Phase 3 spike in [IMPLEMENTATION_PLAN.md](../IMPLEMENTATION_PLAN.md) (two
hours of microphone and loopback on a Windows machine) is still to do, and the checklist at the
end of this document is what it must cover.

## Two tracks, one timeline

A recording has exactly two original tracks, each its own file in its device's native format:

| Track | Device | Notes |
|-------|--------|-------|
| Microphone | A recording device | |
| System audio | A playback device, opened for loopback | Delivers what that device plays, and nothing while nothing is played |

Which device each one is, is a setting (below). They are never mixed into one file and neither is substituted for the other. A derived playback
mix, when one is added, goes in `derived/` and leaves these untouched.

One monotonic clock is started per recording. Every packet a device delivers carries the
device's own timestamp; Skribe converts it to microseconds from the recording's origin and
writes it to the track's clock map. Sample position therefore maps to timeline time without
assuming the nominal sample rate, which is what keeps two devices that drift apart aligned.

Time during which a device delivers nothing is a gap between clock map entries. The audio file
holds only what was delivered: no silence is invented, and no time goes missing.

## Choosing the devices (decision D16)

For each track the user chooses, in Settings, one of:

| Choice | Device recorded |
|--------|-----------------|
| Windows default device | Whatever the default is when recording starts. This is the choice until the user makes another |
| Windows default communications device | Whatever the communications default is when recording starts. Meeting apps commonly play and record through it, and it can differ from the default device |
| A specific device | That device and no other |

- The choice is looked up when recording starts, among the devices that are active then.
- A specific device that is not connected refuses the start, naming the track. Skribe never
  records a different device in its place: a recording of the wrong device looks like a
  recording of the meeting until someone listens to it.
- The settings page lists the connected devices, says which one each default currently is, and
  keeps an unplugged choice in the list marked "not connected".
- The frontend sends only a device's identifier. Rust accepts it only if that device is
  connected now and of the right kind (a playback device cannot be chosen as the microphone),
  and saves the name the system reports.
- A change applies from the next recording. A recording that is running keeps the devices it
  started with.
- Checking devices before a recording shows the device each track would use and why.

## Files

```
library/audio/<meeting id>/
  <track id>.wav       the original audio
  <track id>.clock     each packet's place in the track and on the timeline
  <track id>.journal   what has been made durable, and the track's format
```

### `.wav`

A WAV file with a fixed 104-byte header:

| Bytes | Content |
|-------|---------|
| 0–11 | `RIFF`, size, `WAVE` |
| 12–47 | A `JUNK` chunk of 28 bytes, reserved |
| 48–95 | `fmt ` chunk: `WAVE_FORMAT_EXTENSIBLE` (channels, rate, bits, valid bits, channel mask, PCM or IEEE float sub-format) |
| 96–103 | `data`, size |
| 104– | Samples, exactly as the device delivered them |

A file whose audio passes 4 GB is written as RF64 instead: `RIFF` becomes `RF64`, the reserved
`JUNK` chunk becomes the `ds64` chunk holding the 64-bit sizes, and both 32-bit size fields are
set to all ones. The header is the same length either way, so finalising or repairing a track
rewrites the first 104 bytes and never moves audio. About six hours of 48 kHz stereo float
crosses the limit.

While a track is being recorded its header says it holds no audio. The true length is in the
journal and is written to the header when the track is finalised.

Supported device formats: 16, 24 or 32-bit integer PCM and 32-bit float, 8 to 384 kHz, 1 to 32
channels. Anything else is refused at preflight with the format named.

### `.clock`

A 16-byte header (`SKRCLK01`, sample rate) followed by one 24-byte little-endian entry per
packet:

| Field | Type | Meaning |
|-------|------|---------|
| frame | u64 | Position of the packet's first frame in the track |
| timeline_us | i64 | Timeline time of that frame |
| frames | u32 | Frames in the packet |
| flags | u32 | 1: the device reported a discontinuity. 2: the device marked the packet silent (written as zeros). 4: frames were dropped before this packet. 8: the device gave no timestamp, so the arrival time was used |

At ten-millisecond packets this is about 9 MB per track per hour.

### `.journal`

A 32-byte header (`SKRJNL01`, the track's format, CRC-32) followed by 48-byte records, each
with its own CRC-32: bytes of audio durable, bytes of clock map durable, frames dropped, and
the timeline time of the track's first frame and of the end of its last. A final record marks
the track as finalised.

## While recording

Each track has a capture thread, a fixed-size buffer and a writer thread.

- **Bounded memory.** The buffer holds two seconds of audio and at most 1,024 packets. It is
  allocated once and never grows, however long the recording. A simulated two-hour recording
  of both tracks (`a_long_recording_stays_within_a_fixed_memory_bound`, run on request) held
  the process at 4 to 5 MiB throughout.
- **Overflow.** If the writer falls behind and the buffer is full, the capture thread does not
  wait: the packet is dropped, the frames and the time they covered are counted, a health event
  is saved, the next clock map entry is flagged, and capture continues.
- **Durability.** Once a second the writer flushes the audio to the device, then the clock map,
  then appends a journal record naming how much of each is now durable. A crash loses at most
  what arrived after the last record.
- **Health.** Each track reports its level, frames captured and frames dropped a few times a
  second, and whether its device is still delivering.

### Preflight

A recording starts only if all of these pass; otherwise it is refused with the specific reason
and nothing is recorded or changed:

- the meeting is in `setup`, not in the trash, and no other recording is live;
- the device chosen for each track is connected and opens, and its native format can be
  recorded;
- the library's audio folder can be written;
- the disk has room for at least one hour of both tracks at their formats.

### Losing a device (decision D13)

If a device disappears mid-recording, that track ends at that point on the timeline and is
reported as failed, with the time and the reason. The other track keeps recording. No other
device is switched in. When the recording stops, the lost track is finalised as a valid partial
file, so the meeting is `recorded` and the track carries the failure.

### No pause

There is no pause. The state machine has no edge back into `recording`, and the interface has
no pause control. Stopping asks for confirmation and says the recording cannot be resumed.

### Closing the window (decision D15)

The process exits without running destructors, so a window closed mid-recording would leave
both tracks unfinalised. While a recording is starting, running or being saved, the window
refuses to close and says why. The user stops the recording, which finalises both tracks, and
can then close.

## Stopping and finalising

Both devices are stopped together. Each writer drains its buffer and makes the last of it
durable. Each track is then finalised: the true header is written, the file is flushed, and its
SHA-256 is computed by reading the whole file back. The frame count, timeline offset of the
first frame, health and checksum are saved with the track.

| Each track | Meeting |
|------------|---------|
| Both have a valid file | `recorded`. A track that stopped early is marked failed and holds the audio up to that point |
| Either has no valid file | `failed`. The track that is valid, and everything else about the meeting, is kept |

A track's health is `healthy` (captured from start to stop with nothing lost), `degraded` (a
valid track known to be missing audio: frames were dropped, or it was recovered) or `failed`.

## Interrupted recordings

Only one process can have a library open, so when a library is opened nothing in it can be
recording. A meeting found in `recording` or `finalising` becomes `interrupted`, and the app
says so when it starts. A meeting found in `ready` never started capturing and returns to
`setup`.

Recovery is asked for by the user. For each track it reads the journal, takes the last record
whose checksum holds, cuts the audio and the clock map at the lengths that record names, writes
the true header and computes the checksum. What was written after that record, under one
second, is cut off because nothing confirmed it reached the disk.

- A track that recovers is `recovered` and `degraded`.
- A track that had already been finalised before the interruption is verified and left as it
  was.
- A track whose journal is unreadable, or whose audio is shorter than the journal says, is
  left exactly as it is and reported as failed. Nothing is deleted or overwritten.

Recovery needs no capture device, so it works in any build.

## Builds without capture

Windows audio code is compiled only on Windows. A build for another platform, including the
WSL development build, says "This build of Skribe cannot record audio" when asked to record.
Preparing meetings works everywhere.

The `synthetic-capture` feature of the app crate records a generated test tone instead, so the
workflow can be exercised in WSL:

```
npx tauri dev --features synthetic-capture
```

It lists two generated devices per track, so choosing between them can be exercised. They are
named "Generated test tone (not a microphone)" and the like, and the interface says, in
Settings and before and during recording, that it is not recording real audio. The feature is not enabled by the
installer build, and a release build that enables it does not compile.

## Still to verify on Windows

None of this has run against a real device. The spike must cover:

- [ ] The installed app records both tracks; each file plays in an ordinary audio tool.
- [ ] A two-hour recording of microphone and loopback: memory stays flat; no frames dropped;
      both files and clock maps are complete.
- [ ] Device timestamps: the two tracks stay aligned over two hours (measure drift against a
      signal played and recorded at once); the first packet of each track lands just after zero.
- [ ] Loopback silence: playing nothing produces a gap in the clock map and no packets; whether
      Windows marks packets silent instead on any tested device.
- [ ] Device choice: the list shows every active recording and playback device with the right
      names; each of the three kinds of choice records from the device it names; a device's
      identifier is the same after unplugging and replugging it and after a restart.
- [ ] With different default and communications playback devices, a call in a real meeting
      app: which of the two the app plays through, and whether system audio should follow the
      communications default out of the box.
- [ ] Unplugging the chosen device before starting: the start is refused, names the track,
      and nothing is recorded from another device.
- [ ] Unplugging the microphone, and separately the playback device, mid-recording: the other
      track continues; the lost track is a valid partial file with the right stop time.
- [ ] Microphone access denied in Windows privacy settings gives the access-denied message.
- [ ] Killing the process mid-recording, then recovering: at most one second lost.
- [ ] A recording past 4 GB is written as RF64 and opens in an audio tool.
- [ ] Closing the window mid-recording is refused and says why; logging off Windows
      mid-recording is recoverable afterwards.
- [ ] A drive that is full, removed or slow mid-recording: the track fails with a write error
      and what was saved is kept.
