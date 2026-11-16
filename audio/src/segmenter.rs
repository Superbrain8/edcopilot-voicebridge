//! Cuts a 16 kHz mono sample stream into utterances.

use std::collections::VecDeque;

/// Samples in one analysis frame (30 ms at 16 kHz).
pub const FRAME_SAMPLES: usize = 480;

/// Decides whether one frame holds speech.
pub trait Vad: Send {
    fn is_speech(&mut self, frame: &[f32]) -> bool;
}

/// Speech detector that compares the frame level with a threshold.
pub struct EnergyVad {
    threshold_dbfs: f32,
}

impl EnergyVad {
    /// A frame is speech when its RMS level in dBFS is above `threshold_dbfs`.
    pub fn new(threshold_dbfs: f32) -> EnergyVad {
        EnergyVad { threshold_dbfs }
    }
}

impl Vad for EnergyVad {
    fn is_speech(&mut self, frame: &[f32]) -> bool {
        if frame.is_empty() {
            return false;
        }
        let mean_square = frame.iter().map(|s| s * s).sum::<f32>() / frame.len() as f32;
        let rms = mean_square.sqrt();
        if rms <= 0.0 {
            return false;
        }
        20.0 * rms.log10() > self.threshold_dbfs
    }
}

/// Timing values of the segmenter, in frames of `FRAME_SAMPLES`.
#[derive(Debug, Clone)]
pub struct SegmenterConfig {
    /// Speech frames in sequence that start an utterance.
    pub start_frames: usize,
    /// Frames in sequence without speech that end an utterance.
    pub end_silence_frames: usize,
    /// Frames before the start that go into the utterance.
    pub pre_roll_frames: usize,
    /// An utterance with fewer speech frames is dropped.
    pub min_frames: usize,
    /// An utterance of this many frames ends at once.
    pub max_frames: usize,
}

impl Default for SegmenterConfig {
    fn default() -> Self {
        SegmenterConfig {
            start_frames: 3,
            end_silence_frames: 20,
            pre_roll_frames: 10,
            min_frames: 10,
            max_frames: 334,
        }
    }
}

/// Splits a sample stream into utterances.
pub struct Segmenter {
    vad: Box<dyn Vad>,
    config: SegmenterConfig,
    /// Samples that do not fill a frame yet.
    partial: Vec<f32>,
    /// Idle state: the last frames, with their speech flag.
    ring: VecDeque<(Vec<f32>, bool)>,
    /// Frames with speech in sequence, idle state.
    speech_run: usize,
    /// A started utterance.
    active: Option<Utterance>,
}

struct Utterance {
    samples: Vec<f32>,
    frames: usize,
    speech_frames: usize,
    silence_run: usize,
}

impl Segmenter {
    /// # Panics
    ///
    /// Panics when `start_frames`, `end_silence_frames` or `max_frames` is 0.
    pub fn new(vad: Box<dyn Vad>, config: SegmenterConfig) -> Segmenter {
        assert!(config.start_frames > 0, "start_frames must be at least 1");
        assert!(
            config.end_silence_frames > 0,
            "end_silence_frames must be at least 1"
        );
        assert!(config.max_frames > 0, "max_frames must be at least 1");
        Segmenter {
            vad,
            config,
            partial: Vec::with_capacity(FRAME_SAMPLES),
            ring: VecDeque::new(),
            speech_run: 0,
            active: None,
        }
    }

    /// Takes samples in blocks of any length. Returns the utterances that
    /// ended in this call.
    pub fn push(&mut self, samples: &[f32]) -> Vec<Vec<f32>> {
        let mut out = Vec::new();
        let mut rest = samples;
        while !rest.is_empty() {
            let take = (FRAME_SAMPLES - self.partial.len()).min(rest.len());
            self.partial.extend_from_slice(&rest[..take]);
            rest = &rest[take..];
            if self.partial.len() == FRAME_SAMPLES {
                let frame = std::mem::replace(&mut self.partial, Vec::with_capacity(FRAME_SAMPLES));
                if let Some(utterance) = self.push_frame(frame) {
                    out.push(utterance);
                }
            }
        }
        out
    }

    /// Drops a started utterance, the pre-roll and the partial frame.
    pub fn reset(&mut self) {
        self.partial.clear();
        self.ring.clear();
        self.speech_run = 0;
        self.active = None;
    }

    fn push_frame(&mut self, frame: Vec<f32>) -> Option<Vec<f32>> {
        let speech = self.vad.is_speech(&frame);
        match self.active.as_mut() {
            None => self.push_idle(frame, speech),
            Some(u) => {
                u.samples.extend_from_slice(&frame);
                u.frames += 1;
                if speech {
                    u.speech_frames += 1;
                    u.silence_run = 0;
                } else {
                    u.silence_run += 1;
                }
                if u.silence_run >= self.config.end_silence_frames
                    || u.frames >= self.config.max_frames
                {
                    self.finish()
                } else {
                    None
                }
            }
        }
    }

    fn push_idle(&mut self, frame: Vec<f32>, speech: bool) -> Option<Vec<f32>> {
        let keep = self.config.pre_roll_frames + self.config.start_frames;
        self.ring.push_back((frame, speech));
        while self.ring.len() > keep {
            self.ring.pop_front();
        }
        self.speech_run = if speech { self.speech_run + 1 } else { 0 };
        if self.speech_run < self.config.start_frames {
            return None;
        }
        // The ring ends with the start frames; the frames before are pre-roll.
        while self.ring.len() > self.config.max_frames {
            self.ring.pop_front();
        }
        let mut u = Utterance {
            samples: Vec::with_capacity(self.ring.len() * FRAME_SAMPLES),
            frames: self.ring.len(),
            speech_frames: 0,
            silence_run: 0,
        };
        for (f, s) in self.ring.drain(..) {
            u.samples.extend_from_slice(&f);
            u.speech_frames += usize::from(s);
        }
        self.speech_run = 0;
        let full = u.frames >= self.config.max_frames;
        self.active = Some(u);
        if full {
            self.finish()
        } else {
            None
        }
    }

    /// Ends the started utterance and goes to the idle state.
    fn finish(&mut self) -> Option<Vec<f32>> {
        self.ring.clear();
        self.speech_run = 0;
        let u = self.active.take()?;
        (u.speech_frames >= self.config.min_frames).then_some(u.samples)
    }

    #[cfg(test)]
    fn buffered_samples(&self) -> usize {
        self.partial.len()
            + self.ring.iter().map(|(f, _)| f.len()).sum::<usize>()
            + self.active.as_ref().map_or(0, |u| u.samples.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Gives the scripted answers one per frame, then `false`.
    struct ScriptVad {
        answers: VecDeque<bool>,
    }

    impl Vad for ScriptVad {
        fn is_speech(&mut self, _frame: &[f32]) -> bool {
            self.answers.pop_front().unwrap_or(false)
        }
    }

    /// Builds a script from (is_speech, frame count) parts, and the samples.
    fn script(parts: &[(bool, usize)]) -> (Box<dyn Vad>, Vec<f32>) {
        let mut answers = VecDeque::new();
        let mut samples = Vec::new();
        for &(speech, n) in parts {
            for _ in 0..n {
                answers.push_back(speech);
                samples.extend(std::iter::repeat_n(0.1, FRAME_SAMPLES));
            }
        }
        (Box::new(ScriptVad { answers }), samples)
    }

    fn run(parts: &[(bool, usize)]) -> Vec<Vec<f32>> {
        let (vad, samples) = script(parts);
        let mut seg = Segmenter::new(vad, SegmenterConfig::default());
        seg.push(&samples)
    }

    #[test]
    fn test_silence_gives_nothing() {
        assert!(run(&[(false, 100)]).is_empty());
    }

    #[test]
    fn test_one_utterance() {
        let out = run(&[(false, 10), (true, 30), (false, 25)]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].len(), (10 + 30 + 20) * FRAME_SAMPLES);
    }

    #[test]
    fn test_short_noise_is_dropped() {
        assert!(run(&[(true, 2), (false, 40)]).is_empty());
        assert!(run(&[(true, 5), (false, 40)]).is_empty());
    }

    #[test]
    fn test_pause_shorter_than_the_end_silence_stays_one_utterance() {
        let out = run(&[(true, 15), (false, 10), (true, 15), (false, 25)]);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn test_two_utterances() {
        let out = run(&[(true, 20), (false, 30), (true, 20), (false, 30)]);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn test_speech_that_does_not_stop_is_cut_at_the_maximum() {
        let config = SegmenterConfig::default();
        let (vad, samples) = script(&[(true, 1000)]);
        let mut seg = Segmenter::new(vad, config.clone());
        let mut out = Vec::new();
        for block in samples.chunks(FRAME_SAMPLES) {
            out.extend(seg.push(block));
            assert!(
                seg.buffered_samples()
                    <= (config.max_frames + config.pre_roll_frames) * FRAME_SAMPLES
            );
        }
        assert!(out.len() >= 2);
        for u in &out {
            assert!(u.len() <= config.max_frames * FRAME_SAMPLES);
        }
    }

    #[test]
    fn test_block_size_does_not_change_the_result() {
        let parts = [
            (false, 7),
            (true, 20),
            (false, 30),
            (true, 12),
            (false, 5),
            (true, 12),
            (false, 30),
        ];
        let mut results = Vec::new();
        for block in [480, 100, 1000] {
            let (vad, samples) = script(&parts);
            let mut seg = Segmenter::new(vad, SegmenterConfig::default());
            let mut out = Vec::new();
            for chunk in samples.chunks(block) {
                out.extend(seg.push(chunk));
            }
            results.push(out);
        }
        assert_eq!(results[0].len(), 2);
        assert_eq!(results[0], results[1]);
        assert_eq!(results[0], results[2]);
    }

    #[test]
    fn test_reset_drops_a_started_utterance() {
        let (vad, samples) = script(&[(true, 30), (false, 30)]);
        let mut seg = Segmenter::new(vad, SegmenterConfig::default());
        let head = 30 * FRAME_SAMPLES;
        assert!(seg.push(&samples[..head]).is_empty());
        seg.reset();
        assert_eq!(seg.buffered_samples(), 0);
        assert!(seg.push(&samples[head..]).is_empty());
    }

    #[test]
    #[should_panic(expected = "start_frames")]
    fn test_zero_start_frames_panics() {
        let config = SegmenterConfig {
            start_frames: 0,
            ..SegmenterConfig::default()
        };
        let _ = Segmenter::new(Box::new(EnergyVad::new(-40.0)), config);
    }

    #[test]
    fn test_energy_vad() {
        let mut vad = EnergyVad::new(-40.0);
        assert!(!vad.is_speech(&[0.0; FRAME_SAMPLES]));
        let sine = |amp: f32| -> Vec<f32> {
            (0..FRAME_SAMPLES)
                .map(|i| amp * (i as f32 * 0.2).sin())
                .collect()
        };
        assert!(vad.is_speech(&sine(1.0)));
        assert!(!vad.is_speech(&sine(0.001)));
    }
}
