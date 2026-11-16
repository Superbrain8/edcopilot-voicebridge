//! A scripted engine for tests of other crates.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use crate::{AudioClip, SttEngine, Transcript};

/// Gives the given replies in sequence, then an error.
pub struct FakeEngine {
    replies: VecDeque<anyhow::Result<String>>,
    calls: usize,
    /// The replies of the second pass. `None`: the engine has no second pass.
    biased: Option<VecDeque<anyhow::Result<String>>>,
    /// Shared, so that a test can read it after the engine moved into a session.
    biased_calls: Arc<AtomicUsize>,
}

impl FakeEngine {
    pub fn new(replies: Vec<anyhow::Result<String>>) -> FakeEngine {
        FakeEngine {
            replies: replies.into(),
            calls: 0,
            biased: None,
            biased_calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Gives the engine a second pass with these replies in sequence.
    pub fn with_biased(mut self, replies: Vec<anyhow::Result<String>>) -> FakeEngine {
        self.biased = Some(replies.into());
        self
    }

    /// Number of `transcribe` calls so far.
    pub fn calls(&self) -> usize {
        self.calls
    }

    /// Number of `transcribe_biased` calls so far.
    pub fn biased_calls(&self) -> usize {
        self.biased_calls.load(Ordering::SeqCst)
    }

    /// The counter of the `transcribe_biased` calls. It stays readable after
    /// the engine moved into a session.
    pub fn biased_call_counter(&self) -> Arc<AtomicUsize> {
        Arc::clone(&self.biased_calls)
    }
}

impl SttEngine for FakeEngine {
    fn name(&self) -> &str {
        "fake"
    }

    fn transcribe(&mut self, _clip: &AudioClip) -> anyhow::Result<Transcript> {
        self.calls += 1;
        match self.replies.pop_front() {
            Some(reply) => reply.map(|text| Transcript { text }),
            None => Err(anyhow::anyhow!("no more replies")),
        }
    }

    fn transcribe_biased(&mut self, _clip: &AudioClip) -> Option<anyhow::Result<Transcript>> {
        let replies = self.biased.as_mut()?;
        self.biased_calls.fetch_add(1, Ordering::SeqCst);
        Some(match replies.pop_front() {
            Some(reply) => reply.map(|text| Transcript { text }),
            None => Err(anyhow::anyhow!("no more biased replies")),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AudioClip, SttEngine};

    #[test]
    fn test_fake_gives_replies_in_sequence_then_an_error() {
        let mut engine = FakeEngine::new(vec![Ok("one".into()), Err(anyhow::anyhow!("boom"))]);
        let clip = AudioClip {
            samples: vec![0.0; 10],
        };
        assert_eq!(engine.name(), "fake");
        assert_eq!(engine.transcribe(&clip).unwrap().text, "one");
        assert!(engine
            .transcribe(&clip)
            .unwrap_err()
            .to_string()
            .contains("boom"));
        assert!(engine
            .transcribe(&clip)
            .unwrap_err()
            .to_string()
            .contains("no more replies"));
        assert_eq!(engine.calls(), 3);
        // No biased replies: no second pass.
        assert!(engine.transcribe_biased(&clip).is_none());
    }

    #[test]
    fn test_fake_gives_biased_replies_in_sequence() {
        let mut engine = FakeEngine::new(vec![Ok("one".into())])
            .with_biased(vec![Ok("Sol".into()), Err(anyhow::anyhow!("boom"))]);
        let clip = AudioClip {
            samples: vec![0.0; 10],
        };
        assert_eq!(
            engine.transcribe_biased(&clip).unwrap().unwrap().text,
            "Sol"
        );
        assert!(engine.transcribe_biased(&clip).unwrap().is_err());
        assert!(engine.transcribe_biased(&clip).unwrap().is_err());
        assert_eq!(engine.biased_calls(), 3);
        assert_eq!(engine.calls(), 0);
    }
}
