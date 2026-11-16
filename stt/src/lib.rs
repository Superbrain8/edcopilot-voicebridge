//! Speech-to-text engines behind one interface.

mod fake;
pub mod hotwords;
#[cfg(feature = "cloud")]
mod openai_compat;
mod origin;
#[cfg(feature = "local")]
mod parakeet;
pub mod wav;

pub use origin::{
    base_url_problem, base_url_user_info_problem, base_url_without_secrets, is_loopback_url,
    request_url, url_origin,
};

use std::time::Duration;

pub use fake::FakeEngine;
pub use hotwords::Hotwords;
#[cfg(feature = "cloud")]
pub use openai_compat::{OpenAiCompatConfig, OpenAiCompatEngine};
#[cfg(feature = "local")]
pub use parakeet::ParakeetEngine;

/// The version text of the linked sherpa-onnx library. The function calls
/// into the native library, so a call proves that the library is linked.
/// `None`: the build has no local engine (the feature `local` is off).
pub fn local_engine_version() -> Option<&'static str> {
    #[cfg(feature = "local")]
    {
        Some(sherpa_onnx::version())
    }
    #[cfg(not(feature = "local"))]
    {
        None
    }
}

/// Sample rate that every engine expects, in Hz.
pub const SAMPLE_RATE: u32 = 16_000;

/// Mono audio at 16,000 Hz, samples in the range -1.0 to 1.0.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioClip {
    pub samples: Vec<f32>,
}

impl AudioClip {
    pub fn duration(&self) -> Duration {
        Duration::from_secs_f64(self.samples.len() as f64 / f64::from(SAMPLE_RATE))
    }
}

/// The text that an engine heard.
#[derive(Debug, Clone, PartialEq)]
pub struct Transcript {
    pub text: String,
}

/// A speech-to-text engine.
pub trait SttEngine: Send {
    fn name(&self) -> &str;
    fn transcribe(&mut self, clip: &AudioClip) -> anyhow::Result<Transcript>;

    /// A second transcription of the same clip with a bias toward the
    /// hotwords. The session asks for it only when the text of `transcribe`
    /// gives no command. `None`: the engine has no second pass.
    fn transcribe_biased(&mut self, _clip: &AudioClip) -> Option<anyhow::Result<Transcript>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "local")]
    #[test]
    fn local_engine_version_comes_from_the_linked_library() {
        let version = local_engine_version().expect("feature local is on");
        assert!(!version.is_empty());
    }

    #[cfg(not(feature = "local"))]
    #[test]
    fn local_engine_version_is_none_without_the_local_engine() {
        assert_eq!(local_engine_version(), None);
    }
}
