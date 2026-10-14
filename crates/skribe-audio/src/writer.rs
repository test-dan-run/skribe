//! The track writer: one thread per track that moves audio from the capture buffer to disk.
//!
//! About once a second it flushes the audio, then the clock map, then appends a journal
//! record naming how much of each is now durable. A crash can therefore lose at most what
//! arrived since the last record, and never leaves a track whose length is unknown.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::clock::{self, ClockEntry, flags};
use crate::journal::{Committed, Journal};
use crate::live::{HealthEventKind, StopReason, TrackLive};
use crate::ring::{Consumer, Item};
use crate::{AudioError, AudioFormat, SessionClock, Source, level, wav};

/// How often a track's audio is made durable. This is the most a crash can lose.
pub const FLUSH_INTERVAL: Duration = Duration::from_secs(1);

/// Clock map entries held before a flush is forced early, so the buffer stays bounded even
/// if packets arrive far faster than expected.
const MAX_BUFFERED_ENTRIES: usize = 4_096;

/// The three files of one track.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackFiles {
    /// The original audio.
    pub wav: PathBuf,
    /// What has been made durable; enough to rebuild the track after a crash.
    pub journal: PathBuf,
    /// Each packet's position in the track and on the timeline.
    pub clock: PathBuf,
}

impl TrackFiles {
    /// `<dir>/<stem>.wav` with its `.journal` and `.clock` beside it.
    pub fn in_dir(dir: &Path, stem: &str) -> Self {
        Self {
            wav: dir.join(format!("{stem}.wav")),
            journal: dir.join(format!("{stem}.journal")),
            clock: dir.join(format!("{stem}.clock")),
        }
    }

    pub(crate) fn all(&self) -> [&Path; 3] {
        [&self.wav, &self.journal, &self.clock]
    }
}

/// How a writer ended.
#[derive(Debug)]
pub(crate) struct WriterEnd {
    /// What the journal's last record says is durable.
    pub committed: Committed,
    pub error: Option<AudioError>,
}

pub(crate) struct Writer {
    thread: JoinHandle<WriterEnd>,
}

fn create(path: &Path, contents: &[u8]) -> Result<File, AudioError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(AudioError::io("create", path))?;
    file.write_all(contents)
        .and_then(|()| file.sync_all())
        .map_err(AudioError::io("write", path))?;
    Ok(file)
}

impl Writer {
    /// Creates the track's three files, each with its header already durable, and starts the
    /// thread that fills them. Existing files are never overwritten.
    pub(crate) fn start(
        source: Source,
        files: &TrackFiles,
        format: AudioFormat,
        clock: SessionClock,
        consumer: Consumer,
        live: Arc<TrackLive>,
    ) -> Result<Self, AudioError> {
        // If a later step fails, the files this call created are removed. A name that was
        // already taken makes `create_new` fail before it is recorded here, so a file that
        // was there before is never touched.
        let mut created: Vec<&Path> = Vec::new();
        let opened = (|| {
            let wav = create(&files.wav, &wav::header(&format, 0))?;
            created.push(&files.wav);
            let clock_file = create(&files.clock, &clock::header(&format))?;
            created.push(&files.clock);
            let journal = Journal::create(&files.journal, &format)?;
            created.push(&files.journal);
            Ok((wav, clock_file, journal))
        })();
        let remove_created = |created: &[&Path]| {
            for path in created {
                let _ = fs::remove_file(path);
            }
        };
        let (wav, clock_file, journal) = match opened {
            Ok(files) => files,
            Err(error) => {
                remove_created(&created);
                return Err(error);
            }
        };
        let mut state = State {
            files: files.clone(),
            format,
            clock,
            wav,
            clock_file,
            journal,
            live,
            pending: Committed::default(),
            committed: Committed::default(),
            entries: Vec::with_capacity(MAX_BUFFERED_ENTRIES * clock::ENTRY_LEN as usize),
            after_drop: false,
        };
        match thread::Builder::new()
            .name(format!("skribe-write-{source}"))
            .spawn(move || state.run(consumer))
        {
            Ok(thread) => Ok(Self { thread }),
            Err(source) => {
                remove_created(&created);
                Err(AudioError::Io {
                    action: "start the writer for",
                    path: files.wav.clone(),
                    source,
                })
            }
        }
    }

    /// Waits for the writer to drain its buffer and make the last of it durable.
    pub(crate) fn finish(self, source: Source) -> WriterEnd {
        self.thread.join().unwrap_or_else(|_| WriterEnd {
            committed: Committed::default(),
            error: Some(AudioError::WriterStopped(source)),
        })
    }
}

struct State {
    files: TrackFiles,
    format: AudioFormat,
    clock: SessionClock,
    wav: File,
    clock_file: File,
    journal: Journal,
    live: Arc<TrackLive>,
    /// Totals written so far, including what is not yet flushed.
    pending: Committed,
    /// Totals as of the last journal record.
    committed: Committed,
    /// Encoded clock map entries waiting for the next flush.
    entries: Vec<u8>,
    after_drop: bool,
}

impl State {
    fn run(&mut self, mut consumer: Consumer) -> WriterEnd {
        let mut last_flush = Instant::now();
        let error = loop {
            let wait = FLUSH_INTERVAL.saturating_sub(last_flush.elapsed());
            let drained = consumer.drain(wait.max(Duration::from_millis(1)));
            let open = drained.open;
            if let Err(error) = self.take(drained.items, drained.bytes) {
                break Some(error);
            }
            let due = last_flush.elapsed() >= FLUSH_INTERVAL
                || self.entries.len() >= MAX_BUFFERED_ENTRIES * clock::ENTRY_LEN as usize;
            if due || !open {
                if let Err(error) = self.flush() {
                    break Some(error);
                }
                last_flush = Instant::now();
            }
            if !open {
                break None;
            }
        };
        if let Some(error) = &error {
            // Refuse further packets rather than let the buffer fill behind a dead writer.
            consumer.close();
            self.live.stop(
                self.clock.now_us(),
                StopReason::WriteFailed,
                &error.to_string(),
            );
        }
        WriterEnd {
            committed: self.committed,
            error,
        }
    }

    /// Writes one drain's worth of audio and queues its clock map entries.
    fn take(&mut self, items: &[Item], bytes: &[u8]) -> Result<(), AudioError> {
        let frame_bytes = u64::from(self.format.bytes_per_frame());
        for item in items {
            match *item {
                Item::Packet(meta) => {
                    let entry = ClockEntry {
                        frame: self.pending.data_bytes / frame_bytes,
                        timeline_us: meta.timeline_us,
                        frames: meta.frames,
                        flags: meta.flags
                            | if self.after_drop {
                                flags::AFTER_DROP
                            } else {
                                0
                            },
                    };
                    self.after_drop = false;
                    entry.encode(&mut self.entries);
                    if self.pending.data_bytes == 0 {
                        self.pending.start_us = meta.timeline_us;
                    }
                    self.pending.data_bytes += u64::from(meta.frames) * frame_bytes;
                    self.pending.clock_bytes += clock::ENTRY_LEN;
                    self.pending.end_us =
                        meta.timeline_us + self.format.duration_us(u64::from(meta.frames));
                }
                Item::Dropped {
                    frames,
                    from_us,
                    until_us,
                } => {
                    self.pending.dropped_frames += frames;
                    self.after_drop = true;
                    self.live
                        .event(from_us, HealthEventKind::Overflow { frames, until_us });
                }
            }
        }
        if !bytes.is_empty() {
            self.wav
                .write_all(bytes)
                .map_err(AudioError::io("write", &self.files.wav))?;
            self.live.raise_peak(level::peak(&self.format, bytes));
        }
        Ok(())
    }

    /// Makes everything written so far durable, in the order that keeps the journal true:
    /// audio, then clock map, then the record that names them.
    fn flush(&mut self) -> Result<(), AudioError> {
        if self.pending == self.committed {
            return Ok(());
        }
        self.wav
            .sync_data()
            .map_err(AudioError::io("flush", &self.files.wav))?;
        self.clock_file
            .write_all(&self.entries)
            .and_then(|()| self.clock_file.sync_data())
            .map_err(AudioError::io("write", &self.files.clock))?;
        self.entries.clear();
        self.journal
            .commit(&self.pending)
            .map_err(AudioError::io("write", &self.files.journal))?;
        self.committed = self.pending;
        Ok(())
    }
}
