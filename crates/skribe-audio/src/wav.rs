//! The track container: a WAV file that becomes RF64 when it outgrows 4 GB.
//!
//! The header has a fixed size and reserves a `JUNK` chunk exactly where RF64's `ds64` chunk
//! goes, so finalising or repairing a track rewrites the first [`HEADER_LEN`] bytes and never
//! moves audio. Samples are stored as the device delivered them, described by a
//! `WAVE_FORMAT_EXTENSIBLE` format chunk.

use crate::clock::array;
use crate::{AudioFormat, SampleKind};

/// Where the audio starts in every track file.
pub const HEADER_LEN: u64 = 104;

const DS64_LEN: u32 = 28;
const FMT_LEN: u32 = 40;
/// Bytes of the RIFF chunk that are not audio: everything after the first eight.
const RIFF_OVERHEAD: u64 = HEADER_LEN - 8;
const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;
/// The PCM and IEEE float sub-format identifiers after their first two bytes, which hold the
/// sample kind: `0000xxxx-0000-0010-8000-00AA00389B71`.
const SUBFORMAT_TAIL: [u8; 14] = [
    0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71,
];

/// What a track header says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WavInfo {
    pub format: AudioFormat,
    pub data_bytes: u64,
    pub rf64: bool,
}

/// The pad byte RIFF requires after a chunk of odd length.
pub(crate) fn pad_bytes(data_bytes: u64) -> u64 {
    data_bytes % 2
}

/// The header for `data_bytes` of audio. Zero while a track is being written: the true size
/// is recorded in the journal and written here when the track is finalised.
pub(crate) fn header(format: &AudioFormat, data_bytes: u64) -> [u8; HEADER_LEN as usize] {
    let riff_size = RIFF_OVERHEAD + data_bytes + pad_bytes(data_bytes);
    let rf64 = riff_size > u64::from(u32::MAX);
    let frames = data_bytes / u64::from(format.bytes_per_frame().max(1));

    let mut out = Vec::with_capacity(HEADER_LEN as usize);
    out.extend_from_slice(if rf64 { b"RF64" } else { b"RIFF" });
    out.extend_from_slice(&small(riff_size, rf64));
    out.extend_from_slice(b"WAVE");
    if rf64 {
        out.extend_from_slice(b"ds64");
        out.extend_from_slice(&DS64_LEN.to_le_bytes());
        out.extend_from_slice(&riff_size.to_le_bytes());
        out.extend_from_slice(&data_bytes.to_le_bytes());
        out.extend_from_slice(&frames.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
    } else {
        out.extend_from_slice(b"JUNK");
        out.extend_from_slice(&DS64_LEN.to_le_bytes());
        out.extend_from_slice(&[0; DS64_LEN as usize]);
    }
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&FMT_LEN.to_le_bytes());
    out.extend_from_slice(&WAVE_FORMAT_EXTENSIBLE.to_le_bytes());
    out.extend_from_slice(&format.channels.to_le_bytes());
    out.extend_from_slice(&format.sample_rate.to_le_bytes());
    out.extend_from_slice(&small(format.bytes_per_second(), false));
    out.extend_from_slice(&small(u64::from(format.bytes_per_frame()), false)[..2]);
    out.extend_from_slice(&format.bits_per_sample.to_le_bytes());
    out.extend_from_slice(&22u16.to_le_bytes());
    out.extend_from_slice(&format.valid_bits.to_le_bytes());
    out.extend_from_slice(&format.channel_mask.to_le_bytes());
    let tag: u16 = match format.kind {
        SampleKind::Int => 1,
        SampleKind::Float => 3,
    };
    out.extend_from_slice(&tag.to_le_bytes());
    out.extend_from_slice(&SUBFORMAT_TAIL);
    out.extend_from_slice(b"data");
    out.extend_from_slice(&small(data_bytes, rf64));
    array(&out)
}

/// A 32-bit size field: all ones when the real size is in the `ds64` chunk.
fn small(value: u64, rf64: bool) -> [u8; 4] {
    if rf64 {
        u32::MAX.to_le_bytes()
    } else {
        u32::try_from(value).unwrap_or(u32::MAX).to_le_bytes()
    }
}

/// Reads a header written by [`header`]. Anything else is refused with the reason.
pub fn read_header(bytes: &[u8]) -> Result<WavInfo, String> {
    if bytes.len() < HEADER_LEN as usize {
        return Err("it is shorter than a track header".to_owned());
    }
    let tag = |at: usize| &bytes[at..at + 4];
    let u16_at = |at: usize| u16::from_le_bytes(array(&bytes[at..]));
    let u32_at = |at: usize| u32::from_le_bytes(array(&bytes[at..]));
    let u64_at = |at: usize| u64::from_le_bytes(array(&bytes[at..]));

    let rf64 = match (tag(0), tag(12)) {
        (b"RIFF", b"JUNK") => false,
        (b"RF64", b"ds64") => true,
        _ => return Err("it does not start with a Skribe track header".to_owned()),
    };
    if tag(8) != b"WAVE" || tag(48) != b"fmt " || tag(96) != b"data" {
        return Err("its chunks are not where a Skribe track header puts them".to_owned());
    }
    if u16_at(56) != WAVE_FORMAT_EXTENSIBLE || bytes[82..96] != SUBFORMAT_TAIL {
        return Err("its format chunk is not the kind Skribe writes".to_owned());
    }
    let kind = match u16_at(80) {
        1 => SampleKind::Int,
        3 => SampleKind::Float,
        other => return Err(format!("its sample kind {other} is not PCM or float")),
    };
    let format = AudioFormat {
        channels: u16_at(58),
        sample_rate: u32_at(60),
        bits_per_sample: u16_at(70),
        valid_bits: u16_at(74),
        channel_mask: u32_at(76),
        kind,
    };
    format.validate().map_err(|error| error.to_string())?;
    let data_bytes = if rf64 {
        u64_at(28)
    } else {
        u64::from(u32_at(100))
    };
    Ok(WavInfo {
        format,
        data_bytes,
        rf64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FLOAT_STEREO: AudioFormat = AudioFormat {
        sample_rate: 48_000,
        channels: 2,
        bits_per_sample: 32,
        valid_bits: 32,
        kind: SampleKind::Float,
        channel_mask: 3,
    };
    const PCM24_MONO: AudioFormat = AudioFormat {
        sample_rate: 44_100,
        channels: 1,
        bits_per_sample: 24,
        valid_bits: 24,
        kind: SampleKind::Int,
        channel_mask: 4,
    };

    #[test]
    fn a_small_track_is_an_ordinary_wav_file() {
        let bytes = header(&FLOAT_STEREO, 800);

        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(array(&bytes[4..])), 96 + 800);
        assert_eq!(&bytes[12..16], b"JUNK");
        assert_eq!(u32::from_le_bytes(array(&bytes[100..])), 800);
        // WAVEFORMATEXTENSIBLE: float sub-format, 8 bytes per frame, 384000 bytes per second.
        assert_eq!(u16::from_le_bytes(array(&bytes[56..])), 0xFFFE);
        assert_eq!(u32::from_le_bytes(array(&bytes[64..])), 384_000);
        assert_eq!(u16::from_le_bytes(array(&bytes[68..])), 8);
        assert_eq!(
            bytes[80..96],
            [
                3, 0, 0, 0, 0, 0, 0x10, 0, 0x80, 0, 0, 0xAA, 0, 0x38, 0x9B, 0x71
            ]
        );
        assert_eq!(
            read_header(&bytes),
            Ok(WavInfo {
                format: FLOAT_STEREO,
                data_bytes: 800,
                rf64: false
            })
        );
    }

    #[test]
    fn an_odd_length_data_chunk_counts_its_pad_byte_in_the_riff_size() {
        let bytes = header(&PCM24_MONO, 9);

        assert_eq!(u32::from_le_bytes(array(&bytes[4..])), 96 + 9 + 1);
        assert_eq!(u32::from_le_bytes(array(&bytes[100..])), 9);
        assert_eq!(read_header(&bytes).expect("header").format, PCM24_MONO);
    }

    #[test]
    fn a_track_over_four_gigabytes_becomes_rf64_in_the_same_space() {
        // About six and a half hours of 48 kHz stereo float.
        let data_bytes: u64 = 9_000_000_000;
        let bytes = header(&FLOAT_STEREO, data_bytes);

        assert_eq!(&bytes[..4], b"RF64");
        assert_eq!(u32::from_le_bytes(array(&bytes[4..])), u32::MAX);
        assert_eq!(&bytes[12..16], b"ds64");
        assert_eq!(u64::from_le_bytes(array(&bytes[20..])), 96 + data_bytes);
        assert_eq!(u64::from_le_bytes(array(&bytes[28..])), data_bytes);
        assert_eq!(u64::from_le_bytes(array(&bytes[36..])), data_bytes / 8);
        assert_eq!(u32::from_le_bytes(array(&bytes[100..])), u32::MAX);
        assert_eq!(
            read_header(&bytes),
            Ok(WavInfo {
                format: FLOAT_STEREO,
                data_bytes,
                rf64: true
            })
        );
    }

    #[test]
    fn the_largest_plain_wav_and_the_smallest_rf64_meet_exactly() {
        let largest_plain = u64::from(u32::MAX) - RIFF_OVERHEAD;
        // Stereo float frames are eight bytes, so step down to a whole frame.
        let whole_frames = largest_plain - largest_plain % 8;

        assert_eq!(&header(&FLOAT_STEREO, whole_frames)[..4], b"RIFF");
        assert_eq!(&header(&FLOAT_STEREO, whole_frames + 8)[..4], b"RF64");
    }

    #[test]
    fn files_skribe_did_not_write_are_refused() {
        let good = header(&FLOAT_STEREO, 800);
        assert!(read_header(&good[..50]).is_err());
        let mut other = good;
        other[..4].copy_from_slice(b"FORM");
        assert!(read_header(&other).is_err());
        let mut plain_pcm = good;
        plain_pcm[56..58].copy_from_slice(&1u16.to_le_bytes());
        assert!(read_header(&plain_pcm).is_err());
        let mut odd_kind = good;
        odd_kind[80..82].copy_from_slice(&7u16.to_le_bytes());
        assert!(read_header(&odd_kind).is_err());
    }
}
