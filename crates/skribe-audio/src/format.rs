use std::fmt;

/// Which of the two original tracks a stream is. The two are never substituted for each
/// other, and neither is assumed to be a particular person.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Source {
    /// The default recording device.
    Microphone,
    /// What the computer plays, captured from the default playback device.
    System,
}

impl Source {
    /// Both tracks of a recording, in the order they are started and reported.
    pub const BOTH: [Self; 2] = [Self::Microphone, Self::System];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Microphone => "microphone",
            Self::System => "system",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::BOTH
            .into_iter()
            .find(|source| source.as_str() == text)
    }
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SampleKind {
    /// Signed integer PCM.
    Int,
    /// IEEE floating point.
    Float,
}

impl SampleKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Int => "int",
            Self::Float => "float",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        [Self::Int, Self::Float]
            .into_iter()
            .find(|kind| kind.as_str() == text)
    }
}

/// A device's native sample format. Tracks are written in it unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AudioFormat {
    pub sample_rate: u32,
    pub channels: u16,
    /// Size of one sample as stored: 16, 24 or 32.
    pub bits_per_sample: u16,
    /// Bits of each stored sample that carry signal.
    pub valid_bits: u16,
    pub kind: SampleKind,
    /// Speaker positions as Windows reports them; zero when unspecified.
    pub channel_mask: u32,
}

/// Why a device format cannot be recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsupportedFormat(pub String);

impl fmt::Display for UnsupportedFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AudioFormat {
    /// Accepts the formats Skribe can write and measure: 16, 24 or 32-bit integer PCM and
    /// 32-bit float, at ordinary sample rates and channel counts.
    pub fn validate(&self) -> Result<(), UnsupportedFormat> {
        let unsupported = |what: String| Err(UnsupportedFormat(what));
        if !(8_000..=384_000).contains(&self.sample_rate) {
            return unsupported(format!("a sample rate of {} Hz", self.sample_rate));
        }
        if !(1..=32).contains(&self.channels) {
            return unsupported(format!("{} channels", self.channels));
        }
        let size_ok = match self.kind {
            SampleKind::Int => matches!(self.bits_per_sample, 16 | 24 | 32),
            SampleKind::Float => self.bits_per_sample == 32,
        };
        if !size_ok {
            return unsupported(format!(
                "{}-bit {} samples",
                self.bits_per_sample,
                self.kind.as_str()
            ));
        }
        if self.valid_bits == 0 || self.valid_bits > self.bits_per_sample {
            return unsupported(format!(
                "{} valid bits in {}-bit samples",
                self.valid_bits, self.bits_per_sample
            ));
        }
        Ok(())
    }

    pub fn bytes_per_sample(&self) -> u32 {
        u32::from(self.bits_per_sample / 8)
    }

    pub fn bytes_per_frame(&self) -> u32 {
        self.bytes_per_sample() * u32::from(self.channels)
    }

    pub fn bytes_per_second(&self) -> u64 {
        u64::from(self.bytes_per_frame()) * u64::from(self.sample_rate)
    }

    /// How long `frames` frames last at the nominal sample rate, in microseconds.
    pub fn duration_us(&self, frames: u64) -> i64 {
        let micros = u128::from(frames) * 1_000_000 / u128::from(self.sample_rate.max(1));
        i64::try_from(micros).unwrap_or(i64::MAX)
    }
}

impl fmt::Display for AudioFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} Hz, {} channel{}, {}-bit {}",
            self.sample_rate,
            self.channels,
            if self.channels == 1 { "" } else { "s" },
            self.bits_per_sample,
            self.kind.as_str()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) const FLOAT_STEREO: AudioFormat = AudioFormat {
        sample_rate: 48_000,
        channels: 2,
        bits_per_sample: 32,
        valid_bits: 32,
        kind: SampleKind::Float,
        channel_mask: 3,
    };

    #[test]
    fn sizes_follow_from_the_format() {
        assert_eq!(FLOAT_STEREO.bytes_per_frame(), 8);
        assert_eq!(FLOAT_STEREO.bytes_per_second(), 384_000);
        assert_eq!(FLOAT_STEREO.duration_us(48_000), 1_000_000);
        assert_eq!(FLOAT_STEREO.duration_us(480), 10_000);
        assert_eq!(FLOAT_STEREO.validate(), Ok(()));
    }

    #[test]
    fn rejects_formats_it_cannot_write_or_measure() {
        let cases = [
            AudioFormat {
                sample_rate: 0,
                ..FLOAT_STEREO
            },
            AudioFormat {
                channels: 0,
                ..FLOAT_STEREO
            },
            AudioFormat {
                bits_per_sample: 64,
                valid_bits: 64,
                ..FLOAT_STEREO
            },
            AudioFormat {
                bits_per_sample: 8,
                valid_bits: 8,
                kind: SampleKind::Int,
                ..FLOAT_STEREO
            },
            AudioFormat {
                valid_bits: 0,
                ..FLOAT_STEREO
            },
            AudioFormat {
                bits_per_sample: 24,
                valid_bits: 32,
                kind: SampleKind::Int,
                ..FLOAT_STEREO
            },
        ];
        for format in cases {
            assert!(format.validate().is_err(), "{format:?}");
        }
    }

    #[test]
    fn sources_and_kinds_round_trip_through_text() {
        for source in Source::BOTH {
            assert_eq!(Source::parse(source.as_str()), Some(source));
        }
        assert_eq!(Source::parse("speaker"), None);
        for kind in [SampleKind::Int, SampleKind::Float] {
            assert_eq!(SampleKind::parse(kind.as_str()), Some(kind));
        }
    }
}
