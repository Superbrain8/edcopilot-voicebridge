//! The steps for one utterance, from audio to result.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use edcopilot_voicebridge_core::matcher::normalize;
use edcopilot_voicebridge_core::{
    confusable_hotwords, BlockReason, Match, MatchResult, NotConsumed, Pipeline,
};
use edcopilot_voicebridge_stt::{AudioClip, SttEngine, Transcript};

use crate::wake_word;

/// What happened to one utterance.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Sent {
        text: String,
        token: String,
        confidence: f32,
        /// The match came from a sound-alike variant of the text.
        sound_alike: bool,
        /// The match came from a synonym variant of the text.
        synonym: bool,
        /// The match came from the filler step: some filler words dropped.
        filler: bool,
    },
    DryRun {
        text: String,
        token: String,
        confidence: f32,
        /// The match came from a sound-alike variant of the text.
        sound_alike: bool,
        /// The match came from a synonym variant of the text.
        synonym: bool,
        /// The match came from the filler step: some filler words dropped.
        filler: bool,
    },
    NoMatch {
        text: String,
        /// The cause when a candidate reached the threshold but the program
        /// sends nothing (see `MatchResult::Blocked`). `None` when no
        /// candidate reached the threshold.
        blocked: Option<BlockReason>,
    },
    NoWakeWord {
        text: String,
    },
    Rejected {
        text: String,
        token: String,
        confidence: f32,
        sound_alike: bool,
        synonym: bool,
        filler: bool,
        reason: String,
    },
    NotConsumed {
        text: String,
        token: String,
        withdrawn: bool,
    },
    EngineError {
        message: String,
    },
    SendError {
        text: String,
        token: String,
        message: String,
    },
}

impl Outcome {
    /// One line for the console. It has no line break.
    pub fn log_line(&self) -> String {
        let line = match self {
            Outcome::Sent {
                text,
                token,
                confidence,
                sound_alike,
                synonym,
                filler,
            } => format!(
                "sent \"{text}\" -> {token} {}",
                confidence_text(*confidence, *sound_alike, *synonym, *filler)
            ),
            Outcome::DryRun {
                text,
                token,
                confidence,
                sound_alike,
                synonym,
                filler,
            } => format!(
                "dry run \"{text}\" -> {token} {}",
                confidence_text(*confidence, *sound_alike, *synonym, *filler)
            ),
            Outcome::NoMatch { text, blocked } => match blocked.and_then(blocked_text) {
                Some(reason) => format!("no match for \"{text}\": {reason}"),
                None => format!("no match for \"{text}\""),
            },
            Outcome::NoWakeWord { text } => format!("no wake word in \"{text}\""),
            Outcome::Rejected {
                text,
                token,
                confidence,
                sound_alike,
                synonym,
                filler,
                reason,
            } => format!(
                "rejected \"{text}\" -> {token} {}: {reason}",
                confidence_text(*confidence, *sound_alike, *synonym, *filler)
            ),
            Outcome::NotConsumed {
                text,
                token,
                withdrawn,
            } => {
                let state = if *withdrawn {
                    "withdrawn"
                } else {
                    "could not be withdrawn"
                };
                format!("EDCoPilot did not consume \"{text}\" -> {token}: {state}")
            }
            Outcome::EngineError { message } => format!("speech engine error: {message}"),
            Outcome::SendError {
                text,
                token,
                message,
            } => format!("send failed for \"{text}\" -> {token}: {message}"),
        };
        line.replace(char::is_control, " ")
    }
}

/// The text of a blocked result for the user. The older causes (a tie,
/// required words, the forms of the text, a dropped word) keep the plain
/// "no match" text.
pub const NO_TOKEN_TEXT: &str =
    "blocked: the text is a phrase of a command that the program cannot send";

/// The reason text of a blocked result, or `None` when the plain "no
/// match" text applies.
pub fn blocked_text(reason: BlockReason) -> Option<&'static str> {
    match reason {
        BlockReason::NoToken => Some(NO_TOKEN_TEXT),
        BlockReason::Tie
        | BlockReason::RequiredWords
        | BlockReason::Forms
        | BlockReason::DroppedWord => None,
    }
}

/// "(0.93)", then "(synonym)" for a match through a synonym,
/// "(sound-alike)" for a match through a sound-alike word, and "(filler)"
/// for a match without some filler words.
fn confidence_text(confidence: f32, sound_alike: bool, synonym: bool, filler: bool) -> String {
    let mut text = format!("({confidence:.2})");
    if synonym {
        text.push_str(" (synonym)");
    }
    if sound_alike {
        text.push_str(" (sound-alike)");
    }
    if filler {
        text.push_str(" (filler)");
    }
    text
}

/// Settings for the steps of one utterance.
pub struct SessionOptions {
    pub wake_words: Vec<String>,
    pub exact_single_word: bool,
    pub dry_run: bool,
    /// The hotword phrases of the engine. The second pass may change only
    /// words of these phrases. Empty: the session never asks for a second
    /// pass.
    pub hotwords: Vec<String>,
}

/// What the matcher says about one text, before a send.
enum Decision {
    /// A command that passed every rule. It is not sent yet.
    Accept { text: String, hit: Match },
    /// The matcher refused on purpose. `reason` gives the cause: a tie, a
    /// best candidate that fails its required words, a path that cannot
    /// make its token (each path that the matcher cannot build, see
    /// `MatchResult::Blocked`), two forms of the text with different
    /// tokens, or a dropped filler word next to a number or a name. No
    /// second pass may decide it.
    Blocked { text: String, reason: BlockReason },
    /// No match, a rejected match, no wake word, or an engine error.
    Other(Outcome),
}

/// Turns utterances into command tokens.
pub struct Session {
    engine: Box<dyn SttEngine>,
    pipeline: Arc<Pipeline>,
    options: SessionOptions,
    last_transcribe: Option<Duration>,
    /// The last outcome came from the second pass.
    last_hotwords: bool,
    /// The vocabulary generation of the last confusion check of the hotwords.
    checked_generation: Option<u64>,
    /// A hotword is near a name of the vocabulary: the second pass is off.
    second_pass_off: bool,
    /// Gets each warning line of the second pass. The default prints it on
    /// stderr.
    notes: Box<dyn Fn(String) + Send>,
    /// The stop flag of the runtime. When it is set, no command goes out.
    stop: Option<Arc<AtomicBool>>,
    /// The last outcome is a command that the stop flag kept back.
    halted: bool,
}

impl Session {
    pub fn new(
        engine: Box<dyn SttEngine>,
        pipeline: Arc<Pipeline>,
        options: SessionOptions,
    ) -> Session {
        Session {
            engine,
            pipeline,
            options,
            last_transcribe: None,
            last_hotwords: false,
            checked_generation: None,
            second_pass_off: false,
            notes: Box::new(|line| eprintln!("{line}")),
            stop: None,
            halted: false,
        }
    }

    /// No command goes out after `stop` is set: the flag is read just before
    /// each send, in the first and in the second pass. A dry run does not
    /// change.
    pub fn with_stop(mut self, stop: Arc<AtomicBool>) -> Session {
        self.stop = Some(stop);
        self
    }

    /// True when the stop flag kept back the command of the last outcome.
    /// That outcome is a `SendError` with the message "the session stops".
    pub fn last_halted_by_stop(&self) -> bool {
        self.halted
    }

    /// Sends each warning line of the second pass to `notes`, not to stderr.
    /// The line is the complete text that the console prints.
    pub fn with_notes(mut self, notes: impl Fn(String) + Send + 'static) -> Session {
        self.notes = Box::new(notes);
        self
    }

    /// Time of the transcription in the last `on_utterance`, with the second
    /// pass when it ran. `None` after `on_text`.
    pub fn last_transcribe_time(&self) -> Option<Duration> {
        self.last_transcribe
    }

    /// True when the outcome of the last `on_utterance` came from the second
    /// pass of the engine (the hotwords).
    pub fn last_used_hotwords(&self) -> bool {
        self.last_hotwords
    }

    /// `on_utterance_until` with no time limit.
    pub fn on_utterance(&mut self, clip: &AudioClip) -> Outcome {
        self.on_utterance_until(clip, None, &Instant::now)
    }

    /// Transcribe the clip, check the wake word, then match and send.
    ///
    /// The second pass of the engine (the hotwords) may confirm a command,
    /// never redirect it. The session asks for it only in two cases:
    ///
    /// - The first text gave no candidate and is not empty. The second text
    ///   must give a command.
    /// - The first text gave a rejected match with token A. The second text
    ///   must give a command with token A.
    ///
    /// In both cases the two texts must have the same number of words, and
    /// each changed word must be part of a whole hotword phrase (case and
    /// punctuation ignored). A blocked first text (a tie) and an empty first
    /// text get no second pass. A vocabulary change during the second pass
    /// drops the second result. A command from the second pass is sent only
    /// when `now()` is not after `deadline`. Otherwise the outcome of the
    /// first text stays.
    pub fn on_utterance_until(
        &mut self,
        clip: &AudioClip,
        deadline: Option<Instant>,
        now: &dyn Fn() -> Instant,
    ) -> Outcome {
        self.last_hotwords = false;
        self.halted = false;
        let start = Instant::now();
        let result = self.engine.transcribe(clip);
        let mut transcribe = start.elapsed();
        let first = self.decide_heard(result);
        let first_outcome = match first {
            Decision::Accept { text, hit } => {
                self.last_transcribe = Some(transcribe);
                return self.finish(text, hit);
            }
            Decision::Blocked { text, reason } => {
                self.last_transcribe = Some(transcribe);
                return Outcome::NoMatch {
                    text,
                    blocked: Some(reason),
                };
            }
            Decision::Other(outcome) => outcome,
        };
        // The text that the second text must keep, except for hotwords. A
        // rejected match must also keep its token.
        let wanted = match &first_outcome {
            Outcome::NoMatch { text, .. } if !normalize(text).is_empty() => {
                Some((text.clone(), None))
            }
            Outcome::Rejected { text, token, .. } => Some((text.clone(), Some(token.clone()))),
            _ => None,
        };
        let Some((first_text, first_token)) = wanted else {
            self.last_transcribe = Some(transcribe);
            return first_outcome;
        };
        let Some(generation) = self.second_pass_generation() else {
            self.last_transcribe = Some(transcribe);
            return first_outcome;
        };
        let start = Instant::now();
        let second = self.engine.transcribe_biased(clip);
        transcribe += start.elapsed();
        self.last_transcribe = Some(transcribe);
        let Some(result) = second else {
            return first_outcome;
        };
        let Decision::Accept { text, hit } = self.decide_heard(result) else {
            return first_outcome;
        };
        // The second decision used the vocabulary of now. A reload since the
        // check of the hotwords makes the check invalid.
        if self.pipeline.vocab_generation() != generation {
            (self.notes)(
                "hotword result dropped: the vocabulary changed during the second pass".to_string(),
            );
            return first_outcome;
        }
        let same_token = first_token.as_ref().is_none_or(|token| hit.token == *token);
        if !same_token || !only_hotwords_changed(&first_text, &text, &self.options.hotwords) {
            return first_outcome;
        }
        if deadline.is_some_and(|d| now() > d) {
            (self.notes)("hotword result dropped: the clip is too old to send".to_string());
            return first_outcome;
        }
        self.last_hotwords = true;
        self.finish(text, hit)
    }

    /// The vocabulary generation for a second pass, or `None` when the
    /// session may not ask for one. After a vocabulary change, the session
    /// checks the hotwords against the names again. A hotword that is near a
    /// name switches the second pass off for the rest of the session, with
    /// one warning for each pair. The command words do not change, so the
    /// start check covers them.
    fn second_pass_generation(&mut self) -> Option<u64> {
        if self.options.hotwords.is_empty() || self.second_pass_off {
            return None;
        }
        let (names, generation) = self.pipeline.names_and_generation();
        if self.checked_generation != Some(generation) {
            self.checked_generation = Some(generation);
            let found = confusable_hotwords(&self.options.hotwords, &names);
            for confusion in &found {
                (self.notes)(format!(
                    "warning: hotwords are off: {} after a vocabulary change",
                    confusion.describe()
                ));
            }
            self.second_pass_off = !found.is_empty();
        }
        (!self.second_pass_off).then_some(generation)
    }

    /// The decision for one transcription: check the wake word, then match.
    fn decide_heard(&self, result: anyhow::Result<Transcript>) -> Decision {
        let heard = match result {
            Ok(transcript) => transcript.text,
            Err(e) => {
                return Decision::Other(Outcome::EngineError {
                    message: format!("{e:#}"),
                })
            }
        };
        if heard.trim().is_empty() {
            return Decision::Other(Outcome::NoMatch {
                text: heard,
                blocked: None,
            });
        }
        match wake_word::strip(&heard, &self.options.wake_words) {
            Some(rest) => self.decide(rest),
            None => Decision::Other(Outcome::NoWakeWord { text: heard }),
        }
    }

    /// A typed text as the first pass of `on_utterance` treats a heard text:
    /// the wake word check, then the match and the send. No engine and no
    /// second pass, because a typed text has no audio.
    pub fn on_heard_text(&mut self, text: &str) -> Outcome {
        self.last_transcribe = None;
        self.last_hotwords = false;
        self.halted = false;
        let heard = Ok(Transcript {
            text: text.to_string(),
        });
        match self.decide_heard(heard) {
            Decision::Accept { text, hit } => self.finish(text, hit),
            Decision::Blocked { text, reason } => Outcome::NoMatch {
                text,
                blocked: Some(reason),
            },
            Decision::Other(outcome) => outcome,
        }
    }

    /// Match and send text. No engine and no wake word.
    pub fn on_text(&mut self, text: &str) -> Outcome {
        self.last_transcribe = None;
        self.last_hotwords = false;
        self.halted = false;
        match self.decide(text) {
            Decision::Accept { text, hit } => self.finish(text, hit),
            Decision::Blocked { text, reason } => Outcome::NoMatch {
                text,
                blocked: Some(reason),
            },
            Decision::Other(outcome) => outcome,
        }
    }

    /// Match the text and apply the rules of the session. Nothing is sent.
    fn decide(&self, text: &str) -> Decision {
        let text = text.trim().to_string();
        if text.is_empty() {
            return Decision::Other(Outcome::NoMatch {
                text,
                blocked: None,
            });
        }
        let hit = match self.pipeline.match_result(&text) {
            MatchResult::Match(hit) => hit,
            MatchResult::Blocked(reason) => return Decision::Blocked { text, reason },
            MatchResult::NoCandidate => {
                return Decision::Other(Outcome::NoMatch {
                    text,
                    blocked: None,
                })
            }
        };
        // A one-word phrase must be exact. So must the required words of a
        // command when they are one word (a commodity name, for example).
        let one_word = text.split_whitespace().count() == 1;
        let one_word_inexact = (one_word && (hit.confidence < 1.0 || hit.via_sound_alike))
            || hit.inexact_single_required_word;
        if self.options.exact_single_word && one_word_inexact {
            return Decision::Other(Outcome::Rejected {
                text,
                token: hit.token,
                confidence: hit.confidence,
                sound_alike: hit.via_sound_alike,
                synonym: hit.via_synonym,
                filler: hit.via_filler,
                reason: "a phrase of one word needs an exact match".to_string(),
            });
        }
        Decision::Accept { text, hit }
    }

    /// Send an accepted command, or give the dry-run outcome.
    fn finish(&mut self, text: String, hit: Match) -> Outcome {
        if self.options.dry_run {
            return Outcome::DryRun {
                text,
                token: hit.token,
                confidence: hit.confidence,
                sound_alike: hit.via_sound_alike,
                synonym: hit.via_synonym,
                filler: hit.via_filler,
            };
        }
        // A stop between the match and the send: the command does not go out.
        if self.stop.as_ref().is_some_and(|s| s.load(Ordering::SeqCst)) {
            self.halted = true;
            return Outcome::SendError {
                text,
                token: hit.token,
                message: "the session stops".to_string(),
            };
        }
        // Send the checked hit. A second match could give another token
        // after a vocabulary reload.
        match self.pipeline.send_match(&hit) {
            Ok(()) => Outcome::Sent {
                text,
                token: hit.token,
                confidence: hit.confidence,
                sound_alike: hit.via_sound_alike,
                synonym: hit.via_synonym,
                filler: hit.via_filler,
            },
            Err(e) => match e.downcast_ref::<NotConsumed>() {
                Some(not_consumed) => Outcome::NotConsumed {
                    text,
                    token: hit.token,
                    withdrawn: not_consumed.withdrawn,
                },
                None => Outcome::SendError {
                    text,
                    token: hit.token,
                    message: format!("{e:#}"),
                },
            },
        }
    }
}

/// True when the two texts have the same number of words and each changed
/// word is part of a whole hotword phrase in `second`. Case and punctuation
/// do not count. A hotword of two words must appear with both words: one
/// word of it alone does not cover a change.
pub fn only_hotwords_changed(first: &str, second: &str, hotwords: &[String]) -> bool {
    let a = normalize(first);
    let b = normalize(second);
    if a.len() != b.len() {
        return false;
    }
    let mut covered = vec![false; b.len()];
    for phrase in hotwords.iter().map(|h| normalize(h)) {
        if phrase.is_empty() || phrase.len() > b.len() {
            continue;
        }
        for start in 0..=b.len() - phrase.len() {
            if b[start..start + phrase.len()] == phrase[..] {
                covered[start..start + phrase.len()].fill(true);
            }
        }
    }
    a.iter()
        .zip(&b)
        .zip(&covered)
        .all(|((x, y), &hot)| x == y || hot)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands_source::test_support::{core_dir, fixture_loaded_commands};
    use edcopilot_voicebridge_core::{
        load_sound_alikes, AliasMap, Command, Fillers, Matcher, SoundAlikes, Synonyms, Writer,
    };
    use edcopilot_voicebridge_stt::FakeEngine;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    /// The pipeline on the invented fixture commands (profile mode) with
    /// the default sound-alike table and no vocabulary.
    fn pipeline(request: PathBuf, confirm: Option<Duration>) -> Arc<Pipeline> {
        let root = core_dir();
        let matcher = Matcher::new(
            fixture_loaded_commands(),
            HashMap::new(),
            AliasMap::new(),
            0.8,
        );
        let writer = Writer::with_timeout(
            request,
            Duration::from_millis(200),
            Duration::from_millis(10),
        )
        .confirm_timeout(confirm);
        let table = load_sound_alikes(&root.join("data/sound_alikes.toml")).unwrap();
        Arc::new(Pipeline::new(matcher, writer).with_sound_alikes(table))
    }

    fn options() -> SessionOptions {
        SessionOptions {
            wake_words: Vec::new(),
            exact_single_word: true,
            dry_run: false,
            hotwords: Vec::new(),
        }
    }

    fn session(
        replies: Vec<anyhow::Result<String>>,
        request: &Path,
        options: SessionOptions,
    ) -> Session {
        let engine = Box::new(FakeEngine::new(replies));
        Session::new(engine, pipeline(request.to_path_buf(), None), options)
    }

    fn say(text: &str) -> Vec<anyhow::Result<String>> {
        vec![Ok(text.to_string())]
    }

    fn clip() -> AudioClip {
        AudioClip {
            samples: vec![0.0; 16],
        }
    }

    fn request_in(dir: &tempfile::TempDir) -> PathBuf {
        dir.path().join("EDCoPilot.request.txt")
    }

    #[test]
    fn test_sent() {
        let dir = tempfile::tempdir().unwrap();
        let request = request_in(&dir);
        let mut s = session(say("help tide log"), &request, options());
        match s.on_utterance(&clip()) {
            Outcome::Sent { token, .. } => assert_eq!(token, "Guide:Panel:TideLog"),
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(std::fs::read(&request).unwrap(), b"Guide:Panel:TideLog");
    }

    #[test]
    fn test_no_match_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let request = request_in(&dir);
        let mut s = session(say("mmm gibberish"), &request, options());
        assert!(matches!(s.on_utterance(&clip()), Outcome::NoMatch { .. }));
        assert!(!request.exists());
    }

    #[test]
    fn test_engine_error_is_an_outcome_and_the_session_continues() {
        let dir = tempfile::tempdir().unwrap();
        let request = request_in(&dir);
        let replies = vec![Err(anyhow::anyhow!("boom")), Ok("help tide log".into())];
        let mut s = session(replies, &request, options());
        match s.on_utterance(&clip()) {
            Outcome::EngineError { message } => assert_eq!(message, "boom"),
            other => panic!("unexpected {other:?}"),
        }
        assert!(matches!(s.on_utterance(&clip()), Outcome::Sent { .. }));
    }

    fn hot(words: &[&str]) -> SessionOptions {
        SessionOptions {
            hotwords: words.iter().map(|w| w.to_string()).collect(),
            ..options()
        }
    }

    /// A session with a second pass. The returned counter counts the calls
    /// of the second pass.
    fn session_with_second_pass(
        first: &str,
        second: Vec<anyhow::Result<String>>,
        pipeline: Arc<Pipeline>,
        options: SessionOptions,
    ) -> (Session, Arc<std::sync::atomic::AtomicUsize>) {
        let engine = FakeEngine::new(say(first)).with_biased(second);
        let calls = engine.biased_call_counter();
        (Session::new(Box::new(engine), pipeline, options), calls)
    }

    fn calls(counter: &Arc<std::sync::atomic::AtomicUsize>) -> usize {
        counter.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// A small pipeline: "go to <system>" with the systems Siris, Sirius and Sol,
    /// and two commands that tie on "close hatch".
    fn systems_pipeline(request: &Path) -> Arc<Pipeline> {
        systems_pipeline_with(request, &["Siris", "Sirius", "Sol"])
    }

    /// `systems_pipeline` with other names: one slot for each name.
    fn systems_pipeline_with(request: &Path, names: &[&str]) -> Arc<Pipeline> {
        let cmd = |phrase: &str, token: &str| Command {
            phrase: Some(phrase.to_string()),
            token_template: token.to_string(),
            ..Default::default()
        };
        let commands = vec![
            cmd("go to {TXT:System1}", "Go:{TXT:System1}"),
            cmd("go to {TXT:System2}", "Go:{TXT:System2}"),
            cmd("go to {TXT:System3}", "Go:{TXT:System3}"),
            cmd("close hatch", "A"),
            cmd("[close; shut] hatch", "B"),
        ];
        let mut vocab = HashMap::new();
        for (i, name) in names.iter().enumerate() {
            vocab.insert(format!("System{}", i + 1), vec![name.to_string()]);
        }
        let matcher = Matcher::new(commands, vocab, AliasMap::new(), 0.8);
        let writer = Writer::with_timeout(
            request.to_path_buf(),
            Duration::from_millis(200),
            Duration::from_millis(10),
        )
        .confirm_timeout(None);
        Arc::new(Pipeline::new(matcher, writer))
    }

    #[test]
    fn test_no_second_pass_after_a_command() {
        let dir = tempfile::tempdir().unwrap();
        let request = request_in(&dir);
        let p = pipeline(request.clone(), None);
        let (mut s, counter) =
            session_with_second_pass("help tide log", say("belay"), p, hot(&["log"]));
        match s.on_utterance(&clip()) {
            Outcome::Sent { token, .. } => assert_eq!(token, "Guide:Panel:TideLog"),
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(calls(&counter), 0);
        assert!(!s.last_used_hotwords());
    }

    #[test]
    fn test_no_second_pass_without_hotwords() {
        let dir = tempfile::tempdir().unwrap();
        let p = pipeline(request_in(&dir), None);
        let (mut s, counter) =
            session_with_second_pass("help tide lark", say("help tide log"), p, options());
        assert!(matches!(s.on_utterance(&clip()), Outcome::NoMatch { .. }));
        assert_eq!(calls(&counter), 0);
    }

    /// Only a hotword changed: the second text gives the command.
    #[test]
    fn test_second_pass_confirms_a_changed_hotword() {
        let dir = tempfile::tempdir().unwrap();
        let request = request_in(&dir);
        let p = pipeline(request.clone(), None);
        let (mut s, counter) =
            session_with_second_pass("Help tide lark.", say("help tide Log"), p, hot(&["Log"]));
        match s.on_utterance(&clip()) {
            Outcome::Sent { text, token, .. } => {
                assert_eq!(text, "help tide Log");
                assert_eq!(token, "Guide:Panel:TideLog");
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(calls(&counter), 1);
        assert!(s.last_used_hotwords());
        assert_eq!(std::fs::read(&request).unwrap(), b"Guide:Panel:TideLog");
    }

    /// The second text also changed a word that is not a hotword: the first
    /// outcome stays.
    #[test]
    fn test_second_pass_may_not_change_other_words() {
        let dir = tempfile::tempdir().unwrap();
        let request = request_in(&dir);
        for (first, hotwords) in [
            ("kelp tide lark", &["log"][..]),
            ("help tide lark", &["Sol"][..]),
            ("help lark", &["tide", "log"][..]),
        ] {
            let p = pipeline(request.clone(), None);
            let (mut s, counter) =
                session_with_second_pass(first, say("help tide log"), p, hot(hotwords));
            match s.on_utterance(&clip()) {
                Outcome::NoMatch { text, .. } => assert_eq!(text, first),
                other => panic!("{first}: unexpected {other:?}"),
            }
            assert_eq!(calls(&counter), 1);
            assert!(!s.last_used_hotwords());
        }
        assert!(!request.exists());
    }

    #[test]
    fn test_only_hotwords_changed() {
        // A hotword of two words covers a change only as a whole phrase.
        let two = vec!["sail out".to_string()];
        assert!(only_hotwords_changed(
            "sat cross to Sol",
            "sail out to Sol",
            &two
        ));
        assert!(only_hotwords_changed(
            "set cross to Sol",
            "sail out to Sol",
            &two
        ));
        // One changed word inside the whole hotword.
        assert!(only_hotwords_changed(
            "sail oat to Sol",
            "sail out to Sol",
            &two
        ));
        // One word of the hotword alone does not cover a change, also not
        // in the other order.
        assert!(!only_hotwords_changed("help oat", "help out", &two));
        assert!(!only_hotwords_changed("oat sail", "out sail", &two));
        let hw = vec!["Sol".to_string()];
        assert!(only_hotwords_changed(
            "Sail out to toll.",
            "Sail Out to Sol.",
            &hw
        ));
        assert!(!only_hotwords_changed(
            "Sail out, too soul.",
            "Sail out to Sol.",
            &hw
        ));
        assert!(!only_hotwords_changed(
            "Sail out to",
            "Sail out to Sol",
            &hw
        ));
        assert!(!only_hotwords_changed("call two", "Sol", &hw));
        assert!(only_hotwords_changed(
            "Navigate to Sol.",
            "navigate to sol",
            &hw
        ));
    }

    /// The first text gives a rejected match with Siris. The second text
    /// gives Sirius: the second pass may not redirect, so the result stays
    /// rejected. A second text with Siris confirms the command.
    #[test]
    fn test_second_pass_after_a_rejected_text_needs_the_same_token() {
        let dir = tempfile::tempdir().unwrap();
        let request = request_in(&dir);
        let p = systems_pipeline(&request);
        // "Sirius" is another token and no hotword. "Siris" is the same token,
        // but no hotword either: the beam decoder changed the word, not a
        // hotword. Both keep the rejected outcome.
        for second in ["go to Sirius", "go to Siris"] {
            let (mut s, counter) = session_with_second_pass(
                "go to sirisa",
                say(second),
                Arc::clone(&p),
                hot(&["Sol"]),
            );
            match s.on_utterance(&clip()) {
                Outcome::Rejected { text, token, .. } => {
                    assert_eq!(text, "go to sirisa");
                    assert_eq!(token, "Go:Siris");
                }
                other => panic!("{second}: unexpected {other:?}"),
            }
            assert_eq!(calls(&counter), 1);
            assert!(!s.last_used_hotwords());
        }
        assert!(!request.exists());
    }

    /// The changed word is a hotword and the token stays: the second text
    /// confirms the rejected match.
    #[test]
    fn test_second_pass_confirms_a_rejected_text_with_a_hotword() {
        let dir = tempfile::tempdir().unwrap();
        let request = request_in(&dir);
        let p = systems_pipeline_with(&request, &["Siris", "Sol"]);
        let (mut s, _) =
            session_with_second_pass("go to sirisa", say("go to Siris"), p, hot(&["Siris"]));
        match s.on_utterance(&clip()) {
            Outcome::Sent { token, .. } => assert_eq!(token, "Go:Siris"),
            other => panic!("unexpected {other:?}"),
        }
        assert!(s.last_used_hotwords());
    }

    /// A rejected first text and a rejected second text: the first stays.
    #[test]
    fn test_rejected_twice_keeps_the_first_rejection() {
        let dir = tempfile::tempdir().unwrap();
        let request = request_in(&dir);
        let p = systems_pipeline_with(&request, &["Siris", "Sol"]);
        let (mut s, counter) =
            session_with_second_pass("go to sirisa", say("go to sirise"), p, hot(&["Siris"]));
        match s.on_utterance(&clip()) {
            Outcome::Rejected { text, .. } => assert_eq!(text, "go to sirisa"),
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(calls(&counter), 1);
        assert!(!request.exists());
    }

    /// The first text has the wake word, the second text lost it: the first
    /// outcome stays.
    #[test]
    fn test_second_text_without_the_wake_word_keeps_the_first_outcome() {
        let dir = tempfile::tempdir().unwrap();
        let request = request_in(&dir);
        let opts = SessionOptions {
            wake_words: vec!["copilot".into()],
            ..hot(&["log"])
        };
        let p = pipeline(request.clone(), None);
        let (mut s, counter) =
            session_with_second_pass("copilot help tide lark", say("help tide log"), p, opts);
        match s.on_utterance(&clip()) {
            Outcome::NoMatch { text, .. } => assert_eq!(text, "help tide lark"),
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(calls(&counter), 1);
        assert!(!request.exists());
    }

    /// A dry run with the second pass: nothing is sent, and the line shows
    /// the hotwords mark.
    #[test]
    fn test_second_pass_in_a_dry_run() {
        let dir = tempfile::tempdir().unwrap();
        let request = request_in(&dir);
        let opts = SessionOptions {
            dry_run: true,
            ..hot(&["log"])
        };
        let p = pipeline(request.clone(), None);
        let (mut s, _) = session_with_second_pass("help tide lark", say("help tide log"), p, opts);
        let outcome = s.on_utterance(&clip());
        match &outcome {
            Outcome::DryRun { token, .. } => assert_eq!(token, "Guide:Panel:TideLog"),
            other => panic!("unexpected {other:?}"),
        }
        assert!(s.last_used_hotwords());
        assert!(!request.exists());
        let marks = crate::diagnostics::Marks {
            glitch: false,
            hotwords: s.last_used_hotwords(),
        };
        let line = crate::run::outcome_line(
            &outcome,
            Duration::ZERO,
            Duration::ZERO,
            Duration::ZERO,
            crate::diagnostics::levels(&[0.1]),
            marks,
        );
        assert!(line.contains(", hotwords)"), "{line}");
    }

    /// An empty first text (silence) gets no second pass.
    #[test]
    fn test_no_second_pass_after_silence() {
        let dir = tempfile::tempdir().unwrap();
        let request = request_in(&dir);
        for first in ["", " ... "] {
            let p = pipeline(request.clone(), None);
            let (mut s, counter) =
                session_with_second_pass(first, say("help tide log"), p, hot(&["log"]));
            assert!(matches!(s.on_utterance(&clip()), Outcome::NoMatch { .. }));
            assert_eq!(calls(&counter), 0, "{first:?}");
        }
    }

    /// A vocabulary reload with a name near a hotword switches the second
    /// pass off for the next utterance.
    #[test]
    fn test_reload_with_a_near_name_switches_the_second_pass_off() {
        let dir = tempfile::tempdir().unwrap();
        let request = request_in(&dir);
        let p = systems_pipeline_with(&request, &["Siris", "Sol"]);
        let engine = FakeEngine::new(vec![Ok("go to serious".into()), Ok("go to toast".into())])
            .with_biased(vec![Ok("go to Siris".into()), Ok("go to Siris".into())]);
        let counter = engine.biased_call_counter();
        let mut s = Session::new(Box::new(engine), Arc::clone(&p), hot(&["Siris"]));
        // "serious" -> "Siris" changes only the hotword.
        assert!(matches!(s.on_utterance(&clip()), Outcome::Sent { .. }));
        assert_eq!(calls(&counter), 1);
        let mut vocab = HashMap::new();
        vocab.insert("System1".to_string(), vec!["Siris".to_string()]);
        vocab.insert("System2".to_string(), vec!["Sirius".to_string()]);
        p.set_dynamic_vocab(vocab);
        // "toast" -> "Siris" changes only the hotword, but the check switched
        // the second pass off.
        assert!(matches!(s.on_utterance(&clip()), Outcome::NoMatch { .. }));
        assert_eq!(calls(&counter), 1);
    }

    /// The engine of this test reloads the vocabulary during its second pass.
    struct ReloadingEngine {
        pipeline: Arc<Pipeline>,
    }

    impl SttEngine for ReloadingEngine {
        fn name(&self) -> &str {
            "reloading"
        }

        fn transcribe(&mut self, _clip: &AudioClip) -> anyhow::Result<Transcript> {
            Ok(Transcript {
                text: "go to serious".into(),
            })
        }

        fn transcribe_biased(&mut self, _clip: &AudioClip) -> Option<anyhow::Result<Transcript>> {
            let mut vocab = HashMap::new();
            vocab.insert("System1".to_string(), vec!["Siris".to_string()]);
            vocab.insert("System2".to_string(), vec!["Varnick".to_string()]);
            self.pipeline.set_dynamic_vocab(vocab);
            Some(Ok(Transcript {
                text: "go to Siris".into(),
            }))
        }
    }

    /// Sets the stop flag during a pass, as a Stop button during the
    /// transcription.
    struct StoppingEngine {
        stop: Arc<std::sync::atomic::AtomicBool>,
        first: &'static str,
        /// The second pass. It sets the flag, not the first pass.
        second: Option<&'static str>,
    }

    impl SttEngine for StoppingEngine {
        fn name(&self) -> &str {
            "stopping"
        }

        fn transcribe(&mut self, _clip: &AudioClip) -> anyhow::Result<Transcript> {
            if self.second.is_none() {
                self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
            }
            Ok(Transcript {
                text: self.first.into(),
            })
        }

        fn transcribe_biased(&mut self, _clip: &AudioClip) -> Option<anyhow::Result<Transcript>> {
            let text = self.second?;
            self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
            Some(Ok(Transcript { text: text.into() }))
        }
    }

    fn stopping_session(
        first: &'static str,
        second: Option<&'static str>,
        pipeline: Arc<Pipeline>,
        options: SessionOptions,
    ) -> Session {
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let engine = StoppingEngine {
            stop: Arc::clone(&stop),
            first,
            second,
        };
        Session::new(Box::new(engine), pipeline, options).with_stop(stop)
    }

    #[test]
    fn test_stop_between_the_match_and_the_send_sends_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let request = request_in(&dir);
        let p = pipeline(request.clone(), None);
        let mut s = stopping_session("help tide log", None, p, options());
        match s.on_utterance(&clip()) {
            Outcome::SendError { token, message, .. } => {
                assert_eq!(token, "Guide:Panel:TideLog");
                assert_eq!(message, "the session stops");
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(s.last_halted_by_stop());
        assert!(!request.exists(), "a command went out after the stop");
    }

    #[test]
    fn test_stop_during_the_second_pass_sends_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let request = request_in(&dir);
        let p = systems_pipeline_with(&request, &["Siris", "Sol"]);
        let mut s = stopping_session("go to serious", Some("go to Siris"), p, hot(&["Siris"]));
        assert!(matches!(s.on_utterance(&clip()), Outcome::SendError { .. }));
        assert!(s.last_halted_by_stop());
        assert!(!request.exists(), "a command went out after the stop");
    }

    #[test]
    fn test_stop_does_not_change_a_dry_run() {
        let dir = tempfile::tempdir().unwrap();
        let request = request_in(&dir);
        let p = pipeline(request.clone(), None);
        let dry = SessionOptions {
            dry_run: true,
            ..options()
        };
        let mut s = stopping_session("help tide log", None, p, dry);
        assert!(matches!(s.on_utterance(&clip()), Outcome::DryRun { .. }));
        assert!(!s.last_halted_by_stop());
        assert!(!request.exists());
    }

    /// A vocabulary change between the two passes drops the second result.
    #[test]
    fn test_reload_during_the_second_pass_drops_its_result() {
        let dir = tempfile::tempdir().unwrap();
        let request = request_in(&dir);
        let p = systems_pipeline_with(&request, &["Siris", "Sol"]);
        let engine = ReloadingEngine {
            pipeline: Arc::clone(&p),
        };
        let mut s = Session::new(Box::new(engine), p, hot(&["Siris"]));
        match s.on_utterance(&clip()) {
            Outcome::NoMatch { text, .. } => assert_eq!(text, "go to serious"),
            other => panic!("unexpected {other:?}"),
        }
        assert!(!s.last_used_hotwords());
        assert!(!request.exists());
    }

    /// A tie is a refusal on purpose: the engine gets no second pass.
    #[test]
    fn test_no_second_pass_after_a_tie() {
        let dir = tempfile::tempdir().unwrap();
        let request = request_in(&dir);
        let p = systems_pipeline(&request);
        let (mut s, counter) =
            session_with_second_pass("close hatch", say("shut hatch"), p, hot(&["shut"]));
        match s.on_utterance(&clip()) {
            Outcome::NoMatch { text, .. } => assert_eq!(text, "close hatch"),
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(calls(&counter), 0);
        assert!(!request.exists());
    }

    /// A hotword near a name of the vocabulary switches the second pass off.
    #[test]
    fn test_hotword_near_a_name_switches_the_second_pass_off() {
        let dir = tempfile::tempdir().unwrap();
        let request = request_in(&dir);
        let p = systems_pipeline(&request);
        let (mut s, counter) =
            session_with_second_pass("go to serious", say("go to Siris"), p, hot(&["Siris"]));
        assert!(matches!(s.on_utterance(&clip()), Outcome::NoMatch { .. }));
        assert_eq!(calls(&counter), 0);
    }

    /// A command of the second pass is not sent after the deadline.
    #[test]
    fn test_second_pass_command_respects_the_deadline() {
        let dir = tempfile::tempdir().unwrap();
        let request = request_in(&dir);
        let p = pipeline(request.clone(), None);
        let (mut s, counter) =
            session_with_second_pass("help tide lark", say("help tide log"), p, hot(&["log"]));
        let start = Instant::now();
        let late = || start + Duration::from_secs(10);
        let outcome = s.on_utterance_until(&clip(), Some(start), &late);
        assert!(matches!(outcome, Outcome::NoMatch { .. }), "{outcome:?}");
        assert_eq!(calls(&counter), 1);
        assert!(!s.last_used_hotwords());
        assert!(!request.exists());
    }

    /// A failed second pass keeps the outcome of the first text.
    #[test]
    fn test_failed_second_pass_keeps_the_first_outcome() {
        let dir = tempfile::tempdir().unwrap();
        let request = request_in(&dir);
        for second in [say("mumble"), vec![Err(anyhow::anyhow!("boom"))], say("")] {
            let p = pipeline(request.clone(), None);
            let (mut s, _) = session_with_second_pass("help tide lark", second, p, hot(&["log"]));
            match s.on_utterance(&clip()) {
                Outcome::NoMatch { text, .. } => assert_eq!(text, "help tide lark"),
                other => panic!("unexpected {other:?}"),
            }
            assert!(!s.last_used_hotwords());
        }
        // No wake word in the first text: no second pass.
        let opts = SessionOptions {
            wake_words: vec!["copilot".into()],
            ..hot(&["log"])
        };
        let p = pipeline(request.clone(), None);
        let (mut s, counter) =
            session_with_second_pass("help tide lark", say("copilot help tide log"), p, opts);
        assert!(matches!(
            s.on_utterance(&clip()),
            Outcome::NoWakeWord { .. }
        ));
        assert_eq!(calls(&counter), 0);
        assert!(!request.exists());
    }

    #[test]
    fn test_wake_word_is_required_and_removed() {
        let dir = tempfile::tempdir().unwrap();
        let request = request_in(&dir);
        let replies = vec![
            Ok("help tide log".into()),
            Ok("CoPilot, help tide log".into()),
        ];
        let opts = SessionOptions {
            wake_words: vec!["copilot".into()],
            ..options()
        };
        let mut s = session(replies, &request, opts);
        assert!(matches!(
            s.on_utterance(&clip()),
            Outcome::NoWakeWord { .. }
        ));
        assert!(!request.exists());
        assert!(matches!(s.on_utterance(&clip()), Outcome::Sent { .. }));
    }

    /// A typed text as if the engine heard it: the wake word is required and
    /// removed before the match and before the one-word rule.
    #[test]
    fn test_heard_text_follows_the_wake_word_rules_of_an_utterance() {
        let dir = tempfile::tempdir().unwrap();
        let request = request_in(&dir);
        let opts = SessionOptions {
            wake_words: vec!["copilot".into()],
            dry_run: true,
            ..options()
        };
        let mut s = session(Vec::new(), &request, opts);
        assert!(matches!(
            s.on_heard_text("help tide log"),
            Outcome::NoWakeWord { .. }
        ));
        match s.on_heard_text("Copilot, help tide log") {
            Outcome::DryRun { text, token, .. } => {
                assert_eq!(text, "help tide log");
                assert_eq!(token, "Guide:Panel:TideLog");
            }
            other => panic!("unexpected {other:?}"),
        }
        // "copilot belays" is one word after the wake word.
        match s.on_heard_text("copilot belays") {
            Outcome::Rejected { reason, .. } => assert!(reason.contains("one word")),
            other => panic!("unexpected {other:?}"),
        }
        assert!(matches!(
            s.on_heard_text("copilot"),
            Outcome::NoMatch { .. }
        ));
        // Without wake words the text is matched as it is.
        let mut s = session(
            Vec::new(),
            &request,
            SessionOptions {
                dry_run: true,
                ..options()
            },
        );
        assert!(matches!(
            s.on_heard_text("help tide log"),
            Outcome::DryRun { .. }
        ));
        assert!(!request.exists());
    }

    #[test]
    fn test_wake_word_only_is_no_match() {
        let dir = tempfile::tempdir().unwrap();
        let request = request_in(&dir);
        let opts = SessionOptions {
            wake_words: vec!["copilot".into()],
            ..options()
        };
        let mut s = session(say("copilot"), &request, opts);
        assert!(matches!(s.on_utterance(&clip()), Outcome::NoMatch { .. }));
        assert!(!request.exists());
    }

    // A one-word phrase: "belay" of the fixture. "belays" is 0.83 from it.
    #[test]
    fn test_single_word_needs_an_exact_match() {
        let dir = tempfile::tempdir().unwrap();
        let request = request_in(&dir);
        let replies = vec![Ok("belays".into()), Ok("belay".into())];
        let mut s = session(replies, &request, options());
        match s.on_utterance(&clip()) {
            Outcome::Rejected { reason, .. } => assert!(reason.contains("one word")),
            other => panic!("unexpected {other:?}"),
        }
        assert!(!request.exists());
        assert!(matches!(s.on_utterance(&clip()), Outcome::Sent { .. }));

        let dir = tempfile::tempdir().unwrap();
        let request = request_in(&dir);
        let opts = SessionOptions {
            exact_single_word: false,
            ..options()
        };
        let mut s = session(say("belays"), &request, opts);
        assert!(matches!(s.on_utterance(&clip()), Outcome::Sent { .. }));
    }

    /// A composite search whose item is one word that is not exact follows
    /// the one-word rule ("canvass" is 0.86 from "canvas", "cord" is 0.75
    /// from "cork").
    #[test]
    fn test_one_word_item_needs_an_exact_match() {
        let dir = tempfile::tempdir().unwrap();
        let request = request_in(&dir);
        let replies = vec![
            Ok("dock sell canvass".into()),
            Ok("dock sell cord".into()),
            Ok("dock sell canvas".into()),
        ];
        let mut s = session(replies, &request, options());
        match s.on_utterance(&clip()) {
            Outcome::Rejected { reason, token, .. } => {
                assert!(reason.contains("one word"));
                assert_eq!(token, "SellDock:Canvas");
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(!request.exists());
        assert!(matches!(s.on_utterance(&clip()), Outcome::NoMatch { .. }));
        assert!(!request.exists());
        match s.on_utterance(&clip()) {
            Outcome::Sent { token, .. } => assert_eq!(token, "SellDock:Canvas"),
            other => panic!("unexpected {other:?}"),
        }

        let dir = tempfile::tempdir().unwrap();
        let request = request_in(&dir);
        let opts = SessionOptions {
            exact_single_word: false,
            ..options()
        };
        let mut s = session(say("dock sell canvass"), &request, opts);
        assert!(matches!(s.on_utterance(&clip()), Outcome::Sent { .. }));
    }

    /// The optional words of the suffix ("rate", "offer", "for") do not
    /// hide a one-word item that is not exact.
    #[test]
    fn test_filler_words_do_not_hide_a_fuzzy_item_word() {
        let phrases = [
            "dock sell for cord",
            "dock sell rate cord",
            "dock sell rate for cord",
            "dock sell rate for fork",
            "dock sell offer for cord",
            "dock sell for corks",
            "dock sell rate for canvass",
        ];
        let dir = tempfile::tempdir().unwrap();
        let request = request_in(&dir);
        let replies = phrases.iter().map(|p| Ok(p.to_string())).collect();
        let mut s = session(replies, &request, options());
        for phrase in phrases {
            let outcome = s.on_utterance(&clip());
            assert!(
                matches!(outcome, Outcome::Rejected { .. } | Outcome::NoMatch { .. }),
                "{phrase}: {outcome:?}"
            );
            assert!(!request.exists(), "{phrase}");
        }
    }

    /// The optional words "find" and "seek" do not hide a misheard item
    /// word (the fixture command `[find; seek;] cork`).
    #[test]
    fn test_full_command_with_one_required_word_follows_the_one_word_rule() {
        let dir = tempfile::tempdir().unwrap();
        let request = request_in(&dir);
        let replies = vec![
            Ok("find cord".into()),
            Ok("seek corks".into()),
            Ok("find cork".into()),
        ];
        let mut s = session(replies, &request, options());
        let outcome = s.on_utterance(&clip());
        assert!(matches!(outcome, Outcome::NoMatch { .. }), "{outcome:?}");
        assert!(!request.exists());
        match s.on_utterance(&clip()) {
            Outcome::Rejected { reason, token, .. } => {
                assert!(reason.contains("one word"));
                assert_eq!(token, "SetCargo:Cork");
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(!request.exists());
        match s.on_utterance(&clip()) {
            Outcome::Sent {
                token, confidence, ..
            } => {
                assert_eq!(token, "SetCargo:Cork");
                assert_eq!(confidence, 1.0);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn test_sound_alike_match_is_marked_in_the_line() {
        let dir = tempfile::tempdir().unwrap();
        let request = request_in(&dir);
        // "by" -> "buy" in the composite prefix of the fixture.
        let mut s = session(say("Cheapest by bid cork."), &request, options());
        let outcome = s.on_utterance(&clip());
        match &outcome {
            Outcome::Sent {
                token, sound_alike, ..
            } => {
                assert_eq!(token, "BuyTop:Cork");
                assert!(sound_alike);
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(
            outcome.log_line().ends_with("(0.95) (sound-alike)"),
            "{}",
            outcome.log_line()
        );
    }

    /// A one-word utterance that matches only after a replacement is rejected.
    /// "to seoul" becomes "to sol": the replaced word is the only required
    /// word, so it follows the one-word rule.
    #[test]
    fn test_replaced_single_required_word_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let request = request_in(&dir);
        let command = Command {
            phrase: Some("[row to; to;] {TXT:System1}".to_string()),
            token_template: "Row:{TXT:System1}".to_string(),
            class: edcopilot_voicebridge_core::CommandClass::DynamicName,
            ..Default::default()
        };
        let vocab = HashMap::from([("System1".to_string(), vec!["Sol".to_string()])]);
        let matcher = Matcher::new(vec![command], vocab, AliasMap::new(), 0.8);
        let writer = Writer::new(request.clone()).confirm_timeout(None);
        let table = SoundAlikes::from_pairs([("seoul", "sol")]);
        let pipeline = Arc::new(Pipeline::new(matcher, writer).with_sound_alikes(table));
        let engine = Box::new(FakeEngine::new(vec![
            Ok("to Seoul.".into()),
            Ok("to sol".into()),
        ]));
        let mut s = Session::new(engine, pipeline, options());
        match s.on_utterance(&clip()) {
            Outcome::Rejected {
                token,
                sound_alike,
                reason,
                ..
            } => {
                assert_eq!(token, "Row:Sol");
                assert!(sound_alike);
                assert!(reason.contains("one word"));
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(!request.exists());
        assert!(matches!(s.on_utterance(&clip()), Outcome::Sent { .. }));
    }

    /// The replaced word is in the prefix or is one of two words: no rejection.
    #[test]
    fn test_replaced_prefix_word_still_sends() {
        let dir = tempfile::tempdir().unwrap();
        let request = request_in(&dir);
        let opts = SessionOptions {
            dry_run: true,
            ..options()
        };
        // "by" -> "buy" and "sail" -> "sale" in the composite prefix of
        // the fixture.
        let replies = vec![
            Ok("cheapest by bid cork".into()),
            Ok("dearest sail bid cork".into()),
        ];
        let mut s = session(replies, &request, opts);
        for expected in ["BuyTop:Cork", "SellTop:Cork"] {
            match s.on_utterance(&clip()) {
                Outcome::DryRun {
                    token, sound_alike, ..
                } => {
                    assert_eq!(token, expected);
                    assert!(sound_alike);
                }
                other => panic!("unexpected {other:?}"),
            }
        }
    }

    /// A synonym match keeps its confidence and shows "(synonym)".
    #[test]
    fn test_synonym_match_is_marked_in_the_line() {
        let dir = tempfile::tempdir().unwrap();
        let request = request_in(&dir);
        let command = Command {
            phrase: Some("sail out for home".to_string()),
            token_template: "SailOutForHome".to_string(),
            ..Default::default()
        };
        let matcher = Matcher::new(vec![command], HashMap::new(), AliasMap::new(), 0.8);
        let writer = Writer::new(request.clone()).confirm_timeout(None);
        let synonyms = Synonyms::from_pairs([("navigate", vec!["sail out for"])]);
        let pipeline = Arc::new(Pipeline::new(matcher, writer).with_synonyms(synonyms));
        let engine = Box::new(FakeEngine::new(vec![Ok("Navigate home.".into())]));
        let opts = SessionOptions {
            dry_run: true,
            ..options()
        };
        let mut s = Session::new(engine, pipeline, opts);
        let outcome = s.on_utterance(&clip());
        match &outcome {
            Outcome::DryRun {
                token,
                synonym,
                sound_alike,
                confidence,
                ..
            } => {
                assert_eq!(token, "SailOutForHome");
                assert!(synonym);
                assert!(!sound_alike);
                assert_eq!(*confidence, 1.0);
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(
            outcome.log_line().ends_with("(1.00) (synonym)"),
            "{}",
            outcome.log_line()
        );
        assert!(!request.exists());
    }

    /// A match through the filler step shows "(filler)". When one word
    /// stays, it must be exact.
    #[test]
    fn test_filler_match_is_marked_in_the_line() {
        let dir = tempfile::tempdir().unwrap();
        let request = request_in(&dir);
        let command = Command {
            phrase: Some("windlass".to_string()),
            token_template: "Windlass".to_string(),
            ..Default::default()
        };
        let matcher = Matcher::new(vec![command], HashMap::new(), AliasMap::new(), 0.8);
        let writer = Writer::new(request.clone()).confirm_timeout(None);
        let fillers = Fillers::from_lists(["the"], ["uh"]);
        let pipeline = Arc::new(Pipeline::new(matcher, writer).with_fillers(fillers));
        let engine = Box::new(FakeEngine::new(vec![
            Ok("Uh, windlass.".into()),
            Ok("Uh, windlasz.".into()),
        ]));
        let opts = SessionOptions {
            dry_run: true,
            ..options()
        };
        let mut s = Session::new(engine, pipeline, opts);
        let outcome = s.on_utterance(&clip());
        match &outcome {
            Outcome::DryRun {
                token,
                filler,
                synonym,
                sound_alike,
                ..
            } => {
                assert_eq!(token, "Windlass");
                assert!(filler);
                assert!(!synonym);
                assert!(!sound_alike);
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(
            outcome.log_line().ends_with("(0.95) (filler)"),
            "{}",
            outcome.log_line()
        );
        match s.on_utterance(&clip()) {
            Outcome::Rejected { token, filler, .. } => {
                assert_eq!(token, "Windlass");
                assert!(filler);
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(!request.exists());
    }

    #[test]
    fn test_one_word_sound_alike_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let request = request_in(&dir);
        let command = Command {
            phrase: Some("buy".to_string()),
            token_template: "Buy".to_string(),
            ..Default::default()
        };
        let matcher = Matcher::new(vec![command], HashMap::new(), AliasMap::new(), 0.8);
        let writer = Writer::new(request.clone()).confirm_timeout(None);
        let table = SoundAlikes::from_pairs([("bye", "buy")]);
        let pipeline = Arc::new(Pipeline::new(matcher, writer).with_sound_alikes(table));
        let engine = Box::new(FakeEngine::new(vec![Ok("Bye.".into()), Ok("buy".into())]));
        let mut s = Session::new(engine, pipeline, options());
        match s.on_utterance(&clip()) {
            Outcome::Rejected {
                token, sound_alike, ..
            } => {
                assert_eq!(token, "Buy");
                assert!(sound_alike);
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(!request.exists());
        assert!(matches!(s.on_utterance(&clip()), Outcome::Sent { .. }));
    }

    #[test]
    fn test_dry_run_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let request = request_in(&dir);
        let opts = SessionOptions {
            dry_run: true,
            ..options()
        };
        let mut s = session(say("help tide log"), &request, opts);
        match s.on_utterance(&clip()) {
            Outcome::DryRun { token, .. } => assert_eq!(token, "Guide:Panel:TideLog"),
            other => panic!("unexpected {other:?}"),
        }
        assert!(!request.exists());
    }

    #[test]
    fn test_not_consumed_is_an_outcome_and_the_session_continues() {
        let dir = tempfile::tempdir().unwrap();
        let request = request_in(&dir);
        let replies = vec![Ok("help tide log".into()), Ok("mmm gibberish".into())];
        let engine = Box::new(FakeEngine::new(replies));
        let p = pipeline(request.clone(), Some(Duration::from_millis(100)));
        let mut s = Session::new(engine, p, options());
        match s.on_utterance(&clip()) {
            Outcome::NotConsumed {
                token, withdrawn, ..
            } => {
                assert_eq!(token, "Guide:Panel:TideLog");
                assert!(withdrawn);
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(!request.exists());
        assert!(matches!(s.on_utterance(&clip()), Outcome::NoMatch { .. }));
    }

    #[test]
    fn test_number_words() {
        let dir = tempfile::tempdir().unwrap();
        let request = request_in(&dir);
        // A scaled range (`[1..20, 5]`): the spoken value is a step.
        let mut s = session(say("fog horn volume fifty five"), &request, options());
        match s.on_utterance(&clip()) {
            Outcome::Sent { token, .. } => assert_eq!(token, "Prefs:HornVolume=\"55\""),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn test_last_transcribe_time() {
        let dir = tempfile::tempdir().unwrap();
        let request = request_in(&dir);
        let opts = SessionOptions {
            dry_run: true,
            ..options()
        };
        let mut s = session(say("help tide log"), &request, opts);
        assert_eq!(s.last_transcribe_time(), None);
        s.on_utterance(&clip());
        assert!(s.last_transcribe_time().is_some());
        s.on_text("help tide log");
        assert_eq!(s.last_transcribe_time(), None);
    }

    /// A text that names a command that the program cannot send gives no
    /// match with the cause. The line says why. Nothing is written.
    #[test]
    fn test_blocked_text_without_a_token_gives_the_cause() {
        let dir = tempfile::tempdir().unwrap();
        let request = request_in(&dir);
        let mut s = session(say("lock lantern"), &request, options());
        for outcome in [s.on_text("lock lantern"), s.on_utterance(&clip())] {
            assert_eq!(
                outcome,
                Outcome::NoMatch {
                    text: "lock lantern".into(),
                    blocked: Some(BlockReason::NoToken),
                }
            );
            assert_eq!(
                outcome.log_line(),
                "no match for \"lock lantern\": blocked: the text is a phrase of a command \
                 that the program cannot send"
            );
        }
        assert!(!request.exists());
    }

    /// The older causes of a block keep the plain line.
    #[test]
    fn test_blocked_line_of_the_older_causes_is_plain() {
        for reason in [
            BlockReason::Tie,
            BlockReason::RequiredWords,
            BlockReason::Forms,
            BlockReason::DroppedWord,
        ] {
            let outcome = Outcome::NoMatch {
                text: "close the gate".into(),
                blocked: Some(reason),
            };
            assert_eq!(outcome.log_line(), "no match for \"close the gate\"");
        }
    }

    #[test]
    fn test_log_line_has_text_token_and_confidence() {
        let line = Outcome::Sent {
            text: "help\ntide log".into(),
            token: "Guide:Panel:TideLog".into(),
            confidence: 0.9,
            sound_alike: false,
            synonym: false,
            filler: false,
        }
        .log_line();
        assert!(line.contains("help tide log"));
        assert!(line.contains("Guide:Panel:TideLog"));
        assert!(line.contains("0.90"));
        assert!(!line.contains('\n'));
    }
}
