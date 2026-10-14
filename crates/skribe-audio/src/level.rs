use crate::{AudioFormat, SampleKind};

/// The loudest sample in `bytes`, from 0.0 (silence) to 1.0 (full scale). Used only for the
/// live level indicator; nothing is derived from it.
pub(crate) fn peak(format: &AudioFormat, bytes: &[u8]) -> f32 {
    let size = format.bytes_per_sample() as usize;
    if size == 0 {
        return 0.0;
    }
    let mut peak: f32 = 0.0;
    for sample in bytes.chunks_exact(size) {
        let value = match (format.kind, sample) {
            (SampleKind::Float, [a, b, c, d]) => f32::from_le_bytes([*a, *b, *c, *d]).abs(),
            (SampleKind::Int, [a, b]) => f32::from(i16::from_le_bytes([*a, *b])).abs() / 32_768.0,
            (SampleKind::Int, [a, b, c]) => {
                // Sign-extend the 24-bit sample through the top byte of an i32.
                let wide = i32::from_le_bytes([0, *a, *b, *c]) >> 8;
                (wide as f32).abs() / 8_388_608.0
            }
            (SampleKind::Int, [a, b, c, d]) => {
                (i32::from_le_bytes([*a, *b, *c, *d]) as f32).abs() / 2_147_483_648.0
            }
            _ => 0.0,
        };
        // A NaN sample compares false and is skipped.
        if value > peak {
            peak = value;
        }
    }
    peak.min(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn format(kind: SampleKind, bits: u16) -> AudioFormat {
        AudioFormat {
            sample_rate: 48_000,
            channels: 1,
            bits_per_sample: bits,
            valid_bits: bits,
            kind,
            channel_mask: 0,
        }
    }

    fn close(found: f32, expected: f32) {
        assert!((found - expected).abs() < 1e-4, "{found} vs {expected}");
    }

    #[test]
    fn measures_each_supported_sample_format() {
        let float: Vec<u8> = [0.1f32, -0.75, 0.5]
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect();
        close(peak(&format(SampleKind::Float, 32), &float), 0.75);

        let int16: Vec<u8> = [1_000i16, -16_384, 200]
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect();
        close(peak(&format(SampleKind::Int, 16), &int16), 0.5);

        // -4194304 is half of negative full scale in 24 bits.
        let int24 = [0x00, 0x00, 0xC0, 0x10, 0x00, 0x00];
        close(peak(&format(SampleKind::Int, 24), &int24), 0.5);

        let int32: Vec<u8> = [i32::MIN / 4, 5]
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect();
        close(peak(&format(SampleKind::Int, 32), &int32), 0.25);
    }

    #[test]
    fn silence_is_zero_and_overs_are_clamped() {
        assert_eq!(peak(&format(SampleKind::Int, 16), &[0; 64]), 0.0);
        assert_eq!(peak(&format(SampleKind::Float, 32), &[]), 0.0);
        let over: Vec<u8> = [3.5f32, f32::NAN]
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect();
        assert_eq!(peak(&format(SampleKind::Float, 32), &over), 1.0);
    }
}
