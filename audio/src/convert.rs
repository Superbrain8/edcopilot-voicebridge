//! Channel mix and sample rate conversion to 16 kHz mono.

use crate::SAMPLE_RATE;

/// Converts interleaved microphone blocks to 16 kHz mono with linear
/// interpolation.
///
/// Output sample `k` is the input signal at time `k / 16000` seconds.
/// The resampler keeps its position between calls, so block borders make
/// no click and the block size does not change the result.
pub struct Resampler {
    input_rate: u32,
    channels: usize,
    /// Samples of an incomplete frame at the end of the last block.
    partial: Vec<f32>,
    /// Mono input samples not yet used up. `buf[0]` has index `base`.
    buf: Vec<f32>,
    base: u64,
    /// Index of the next output sample.
    next_out: u64,
}

impl Resampler {
    /// Creates a resampler for interleaved input at `input_rate` Hz.
    ///
    /// # Panics
    ///
    /// Panics if `input_rate` or `channels` is 0.
    pub fn new(input_rate: u32, channels: u16) -> Resampler {
        assert!(input_rate > 0, "input_rate must not be 0");
        assert!(channels > 0, "channels must not be 0");
        Resampler {
            input_rate,
            channels: usize::from(channels),
            partial: Vec::new(),
            buf: Vec::new(),
            base: 0,
            next_out: 0,
        }
    }

    /// Mixes `interleaved` to mono and appends 16 kHz samples to `out`.
    ///
    /// The block can have any length. A partial frame at the end is kept
    /// and completed by the next block. `out` is not cleared.
    pub fn push(&mut self, interleaved: &[f32], out: &mut Vec<f32>) {
        let ch = self.channels;
        let mut rest = interleaved;

        if !self.partial.is_empty() {
            let need = ch - self.partial.len();
            let take = need.min(rest.len());
            self.partial.extend_from_slice(&rest[..take]);
            rest = &rest[take..];
            if self.partial.len() < ch {
                return;
            }
            let mono = self.partial.iter().sum::<f32>() / ch as f32;
            self.buf.push(mono);
            self.partial.clear();
        }

        let frames = rest.chunks_exact(ch);
        self.partial.extend_from_slice(frames.remainder());
        self.buf
            .extend(frames.map(|f| f.iter().sum::<f32>() / ch as f32));

        self.emit(out);
    }

    fn emit(&mut self, out: &mut Vec<f32>) {
        let rate = u64::from(SAMPLE_RATE);
        let in_rate = u64::from(self.input_rate);
        loop {
            let pos = self.next_out * in_rate;
            let index = pos / rate;
            let rem = pos % rate;
            let Some(i) = index.checked_sub(self.base) else {
                break;
            };
            let i = i as usize;
            let sample = if rem == 0 {
                match self.buf.get(i) {
                    Some(&a) => a,
                    None => break,
                }
            } else {
                match (self.buf.get(i), self.buf.get(i + 1)) {
                    (Some(&a), Some(&b)) => {
                        let t = rem as f32 / rate as f32;
                        a + (b - a) * t
                    }
                    _ => break,
                }
            };
            out.push(sample);
            self.next_out += 1;
        }

        // Drop input samples that no later output sample needs.
        let keep_from = self.next_out * in_rate / rate;
        if keep_from > self.base {
            let drop = ((keep_from - self.base) as usize).min(self.buf.len());
            self.buf.drain(..drop);
            self.base += drop as u64;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(rate: u32, channels: u16, input: &[f32]) -> Vec<f32> {
        let mut r = Resampler::new(rate, channels);
        let mut out = Vec::new();
        r.push(input, &mut out);
        out
    }

    fn sine(freq: f32, rate: u32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / rate as f32).sin())
            .collect()
    }

    fn assert_len_near(actual: usize, expected: usize, tolerance: usize) {
        assert!(
            actual.abs_diff(expected) <= tolerance,
            "length {actual}, expected {expected} +- {tolerance}"
        );
    }

    #[test]
    fn test_16k_mono_passes_through() {
        let input = sine(300.0, 16_000, 1000);
        assert_eq!(run(16_000, 1, &input), input);
    }

    #[test]
    fn test_stereo_is_mixed_to_the_mean() {
        let input = [1.0, 0.0, 1.0, 0.0, 0.5, 0.5];
        assert_eq!(run(16_000, 2, &input), vec![0.5, 0.5, 0.5]);
    }

    #[test]
    fn test_48k_gives_one_third_of_the_samples() {
        let out = run(48_000, 1, &vec![0.1; 48_000]);
        assert_len_near(out.len(), 16_000, 1);
    }

    #[test]
    fn test_44100_length() {
        let out = run(44_100, 1, &vec![0.1; 44_100]);
        assert_len_near(out.len(), 16_000, 1);
    }

    #[test]
    fn test_8k_upsamples_to_twice_the_samples() {
        let out = run(8_000, 1, &vec![0.1; 8_000]);
        assert_len_near(out.len(), 16_000, 2);
    }

    fn run_in_blocks(input: &[f32], channels: u16, block: usize) -> Vec<f32> {
        let mut r = Resampler::new(48_000, channels);
        let mut out = Vec::new();
        for chunk in input.chunks(block) {
            r.push(chunk, &mut out);
        }
        out
    }

    #[test]
    fn test_block_size_does_not_change_the_result() {
        let input = sine(440.0, 48_000, 48_000);
        let whole = run(48_000, 1, &input);
        for block in [480, 479] {
            let blocks = run_in_blocks(&input, 1, block);
            assert_eq!(blocks.len(), whole.len(), "block {block}");
            for (a, b) in whole.iter().zip(&blocks) {
                assert!((a - b).abs() < 1e-6, "block {block}: {a} vs {b}");
            }
        }
    }

    #[test]
    fn test_odd_blocks_of_stereo_keep_partial_frames() {
        let mono = sine(440.0, 48_000, 9_600);
        let input: Vec<f32> = mono.iter().flat_map(|&s| [s, s * 0.5]).collect();
        let whole = run(48_000, 2, &input);
        let blocks = run_in_blocks(&input, 2, 481);
        assert_eq!(blocks.len(), whole.len());
        for (a, b) in whole.iter().zip(&blocks) {
            assert!((a - b).abs() < 1e-6, "{a} vs {b}");
        }
    }

    #[test]
    fn test_push_appends_to_out() {
        let mut r = Resampler::new(16_000, 1);
        let mut out = vec![9.0];
        r.push(&[1.0, 2.0], &mut out);
        assert_eq!(out, vec![9.0, 1.0, 2.0]);
    }

    #[test]
    fn test_sine_keeps_its_frequency() {
        let out = run(48_000, 1, &sine(440.0, 48_000, 48_000));
        let crossings = out.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
        assert!(
            (438..=442).contains(&crossings),
            "{crossings} zero crossings"
        );
    }

    #[test]
    #[should_panic(expected = "channels must not be 0")]
    fn test_zero_channels_panics() {
        Resampler::new(16_000, 0);
    }

    #[test]
    #[should_panic(expected = "input_rate must not be 0")]
    fn test_zero_rate_panics() {
        Resampler::new(0, 1);
    }
}
