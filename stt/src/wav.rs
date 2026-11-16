//! WAV helpers, 16 kHz mono: 16-bit PCM for the cloud upload, 32-bit float
//! for saved clips.

use std::io::Cursor;
use std::path::Path;

use anyhow::{bail, Context};

use crate::{AudioClip, SAMPLE_RATE};

/// Encode a clip as a 32-bit float WAV file in memory. The file keeps each
/// sample bit for bit, so that `read_file` gives the engine the same
/// samples as the live run. A 16-bit file does not: the model can give a
/// different text for a change below one 16-bit step.
pub fn encode_f32(clip: &AudioClip) -> Vec<u8> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut buffer = Cursor::new(Vec::new());
    {
        let mut writer = hound::WavWriter::new(&mut buffer, spec).expect("WAV header");
        for &sample in &clip.samples {
            writer.write_sample(sample).expect("write to memory");
        }
        writer.finalize().expect("finish WAV");
    }
    buffer.into_inner()
}

/// Encode a clip as a 16-bit PCM WAV file in memory.
pub fn encode(clip: &AudioClip) -> Vec<u8> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut buffer = Cursor::new(Vec::new());
    {
        let mut writer = hound::WavWriter::new(&mut buffer, spec).expect("WAV header");
        for &sample in &clip.samples {
            let value = (sample.clamp(-1.0, 1.0) * 32767.0).round() as i16;
            writer.write_sample(value).expect("write to memory");
        }
        writer.finalize().expect("finish WAV");
    }
    buffer.into_inner()
}

/// Read a 16 kHz mono WAV file with 32-bit float or 16-bit integer samples.
/// Float samples come back bit for bit. A 16-bit value `v` becomes `v / 32768`.
pub fn read_file(path: &Path) -> anyhow::Result<AudioClip> {
    let mut reader =
        hound::WavReader::open(path).with_context(|| format!("cannot read {}", path.display()))?;
    let spec = reader.spec();
    if spec.sample_rate != SAMPLE_RATE || spec.channels != 1 {
        bail!(
            "{}: expected 16000 Hz mono, found {} Hz with {} channel(s)",
            path.display(),
            spec.sample_rate,
            spec.channels
        );
    }
    let samples = match (spec.sample_format, spec.bits_per_sample) {
        (hound::SampleFormat::Float, 32) => reader.samples::<f32>().collect::<Result<Vec<f32>, _>>(),
        (hound::SampleFormat::Int, 16) => reader
            .samples::<i16>()
            .map(|s| s.map(|v| f32::from(v) / 32768.0))
            .collect::<Result<Vec<f32>, _>>(),
        (format, bits) => bail!(
            "{}: expected 32-bit float or 16-bit integer samples, found {format:?} with {bits} bits",
            path.display()
        ),
    }
    .with_context(|| format!("cannot read samples of {}", path.display()))?;
    Ok(AudioClip { samples })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("voicebridge-stt-{}-{name}", std::process::id()))
    }

    #[test]
    fn test_encode_then_read_gives_the_same_samples() {
        let samples: Vec<f32> = (0..1600)
            .map(|i| 0.5 * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 16_000.0).sin())
            .collect();
        let clip = AudioClip { samples };
        let path = temp_path("roundtrip.wav");
        std::fs::write(&path, encode(&clip)).unwrap();
        let back = read_file(&path).unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!(back.samples.len(), clip.samples.len());
        for (a, b) in clip.samples.iter().zip(&back.samples) {
            assert!((a - b).abs() <= 1.0 / 32768.0, "{a} vs {b}");
        }
    }

    /// Values that a live clip can hold, also outside -1.0 to 1.0.
    fn awkward_samples() -> Vec<f32> {
        let mut samples: Vec<f32> = (0..1600)
            .map(|i| 0.3 * (i as f32 * 0.37).sin() + 1e-7 * i as f32)
            .collect();
        samples.extend([
            0.0,
            -0.0,
            f32::MIN_POSITIVE,
            1e-40, // subnormal
            1.0 / 3.0,
            -1.0,
            1.0,
            1.25,
            -1.5,
        ]);
        samples
    }

    #[test]
    fn test_float_encode_then_read_is_bit_equal() {
        let clip = AudioClip {
            samples: awkward_samples(),
        };
        let path = temp_path("float.wav");
        std::fs::write(&path, encode_f32(&clip)).unwrap();
        let back = read_file(&path).unwrap();
        std::fs::remove_file(&path).ok();
        let bits = |s: &[f32]| s.iter().map(|v| v.to_bits()).collect::<Vec<u32>>();
        assert_eq!(bits(&back.samples), bits(&clip.samples));
    }

    #[test]
    fn test_read_rejects_float_with_other_widths() {
        let path = temp_path("float64.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: SAMPLE_RATE,
            bits_per_sample: 24,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&path, spec).unwrap();
        writer.write_sample(0i32).unwrap();
        writer.finalize().unwrap();
        let err = read_file(&path).unwrap_err().to_string();
        std::fs::remove_file(&path).ok();
        assert!(err.contains("24"), "{err}");
    }

    #[test]
    fn test_read_rejects_other_formats() {
        let path = temp_path("stereo.wav");
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 44_100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&path, spec).unwrap();
        for _ in 0..100 {
            writer.write_sample(0i16).unwrap();
        }
        writer.finalize().unwrap();
        let err = read_file(&path).unwrap_err().to_string();
        std::fs::remove_file(&path).ok();
        assert!(err.contains("44100"), "{err}");
        assert!(err.contains('2'), "{err}");
    }

    /// A 16-bit file as a recorder writes it (the repository has no
    /// recordings): 1.5 s of a tone, 16 kHz mono, with the extreme values.
    #[test]
    fn test_read_a_16_bit_recording() {
        let path = temp_path("recording.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: SAMPLE_RATE,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut values: Vec<i16> = (0..24_000 - 4)
            .map(|i| {
                let t = i as f32 / SAMPLE_RATE as f32;
                (12_000.0 * (2.0 * std::f32::consts::PI * 220.0 * t).sin()).round() as i16
            })
            .collect();
        values.extend([i16::MIN, i16::MAX, 0, -1]);
        let mut writer = hound::WavWriter::create(&path, spec).unwrap();
        for &v in &values {
            writer.write_sample(v).unwrap();
        }
        writer.finalize().unwrap();

        let header = hound::WavReader::open(&path).unwrap().spec();
        assert_eq!(header.sample_rate, 16_000);
        assert_eq!(header.channels, 1);
        let clip = read_file(&path).unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!(clip.samples.len(), 24_000);
        assert_eq!(clip.duration().as_secs_f64(), 1.5);
        for (v, s) in values.iter().zip(&clip.samples) {
            assert_eq!(*s, f32::from(*v) / 32768.0);
        }
        assert_eq!(clip.samples[23_996], -1.0);
        assert_eq!(clip.samples[23_997], 32767.0 / 32768.0);
    }

    #[test]
    fn test_read_rejects_another_sample_rate() {
        let path = temp_path("8khz.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 8_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&path, spec).unwrap();
        writer.write_sample(0i16).unwrap();
        writer.finalize().unwrap();
        let err = read_file(&path).unwrap_err().to_string();
        std::fs::remove_file(&path).ok();
        assert!(err.contains("8000"), "{err}");
    }
}
