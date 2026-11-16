use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;

use crate::fillers::{filler_variants_with, number_sound_alike, tokens_in_names, Dropped, Fillers};
use crate::matcher::{normalize, BlockReason, Match, MatchResult, Matcher};
use crate::numbers::{is_number_word, number_readings};
use crate::sound_alikes::{sound_alike_variants, SoundAlikes};
use crate::synonyms::{synonym_variants_spoken, Synonyms};
use crate::writer::Writer;

/// The confidence of a match through a sound-alike variant is multiplied by
/// this factor. The match also has `via_sound_alike` set, which the result
/// line and the utterance log show.
pub const SOUND_ALIKE_FACTOR: f32 = 0.95;

/// The confidence of a match through the filler step is multiplied by this
/// factor. The match also has `via_filler` set.
pub const FILLER_FACTOR: f32 = 0.95;

/// The largest number of variants of the filler step over all forms of the
/// text. More variants give no filler step at all.
pub const MAX_FILLER_STEP_VARIANTS: usize = 32;

/// Text in, request file out: matches spoken text and delivers the token.
pub struct Pipeline {
    matcher: RwLock<Matcher>,
    writer: Writer,
    sound_alikes: SoundAlikes,
    synonyms: Synonyms,
    fillers: Fillers,
    vocab_generation: AtomicU64,
}

impl Pipeline {
    pub fn new(matcher: Matcher, writer: Writer) -> Pipeline {
        Pipeline {
            matcher: RwLock::new(matcher),
            writer,
            sound_alikes: SoundAlikes::default(),
            synonyms: Synonyms::default(),
            fillers: Fillers::default(),
            vocab_generation: AtomicU64::new(0),
        }
    }

    /// Sets the synonym table. The default table is empty.
    pub fn with_synonyms(mut self, synonyms: Synonyms) -> Pipeline {
        self.synonyms = synonyms;
        self
    }

    /// Sets the filler words. The default table is empty: no filler step.
    pub fn with_fillers(mut self, fillers: Fillers) -> Pipeline {
        self.fillers = fillers;
        self
    }

    /// Sets the sound-alike table. The default table is empty.
    pub fn with_sound_alikes(mut self, sound_alikes: SoundAlikes) -> Pipeline {
        self.sound_alikes = sound_alikes;
        self
    }

    /// Replaces the vocabulary of the matcher. Safe to call while another thread matches.
    /// The generation changes while the write lock is held, so that
    /// `names_and_generation` always gives a matching pair.
    pub fn set_dynamic_vocab(&self, dynamic_vocab: HashMap<String, Vec<String>>) {
        let mut matcher = self.matcher.write().unwrap_or_else(|e| e.into_inner());
        matcher.set_dynamic_vocab(dynamic_vocab);
        self.vocab_generation.fetch_add(1, Ordering::SeqCst);
    }

    /// Matches the text without a write. The steps are:
    ///
    /// 1. the text as heard, then its number-word variants;
    /// 2. each synonym variant, with its number variants;
    /// 3. each sound-alike variant of the text as heard;
    /// 4. each sound-alike variant of each synonym variant;
    /// 5. the filler step: the forms of steps 1 to 4 without some of their
    ///    filler words (see `match_fillers`).
    ///
    /// The next step runs only when the steps before found no candidate. A
    /// blocked result (a tie, a best candidate that fails its required
    /// words, or a path that cannot make its token, see
    /// `MatchResult::Blocked`) at any step ends the search with no match. Thus another form
    /// of the text never decides a tie. A sound-alike candidate that the
    /// factor puts below the threshold also ends the search.
    ///
    /// In one step, two variants with different tokens give no match, and
    /// for one token the highest confidence stays. A synonym match keeps its
    /// confidence and has `via_synonym` set. A sound-alike match gets
    /// `SOUND_ALIKE_FACTOR` on its confidence and must still reach the
    /// threshold. When a sound-alike replacement is the only required word
    /// or a slot name of one word, the match is marked as an inexact single
    /// required word.
    pub fn match_text(&self, text: &str) -> Option<Match> {
        self.match_result(text).into_match()
    }

    /// The result of `match_text` with the reason of a miss: `Blocked` when
    /// a step was blocked, with the cause (see `BlockReason`: a tie, a best
    /// candidate that fails its required words, a path that cannot make its
    /// token, two forms of the text with different tokens, or a dropped
    /// filler word next to a number or a name), `NoCandidate` when no step
    /// found a candidate that reached the threshold.
    pub fn match_result(&self, text: &str) -> MatchResult {
        let matcher = self.matcher.read().unwrap_or_else(|e| e.into_inner());
        let synonym_texts = synonym_variants_spoken(text, &self.synonyms);
        match self.match_without_fillers(&matcher, text, &synonym_texts) {
            Some(result) => result,
            None => self.match_fillers(&matcher, text, &synonym_texts),
        }
    }

    /// Steps 1 to 4 of `match_text`. `None` when no step found a candidate.
    fn match_without_fillers(
        &self,
        matcher: &Matcher,
        text: &str,
        synonym_texts: &[(String, Vec<bool>)],
    ) -> Option<MatchResult> {
        match match_with_numbers(matcher, text) {
            Step::Found(hit, _) => return Some(MatchResult::Match(hit)),
            Step::Blocked(reason) => return Some(MatchResult::Blocked(reason)),
            Step::Nothing => {}
        }
        let step = synonym_texts.iter().map(|(v, _)| Variant {
            text: v.clone(),
            base: None,
        });
        match best_of(matcher, step) {
            Step::Found(mut hit, _) => {
                hit.via_synonym = true;
                return Some(MatchResult::Match(hit));
            }
            Step::Blocked(reason) => return Some(MatchResult::Blocked(reason)),
            Step::Nothing => {}
        }
        let original = normalize(text);
        let step = sound_alike_variants(text, &self.sound_alikes)
            .into_iter()
            .map(|v| Variant {
                text: v,
                base: Some(original.clone()),
            });
        let (mut hit, via_synonym) = match best_of(matcher, step) {
            Step::Found(hit, _) => (hit, false),
            Step::Blocked(reason) => return Some(MatchResult::Blocked(reason)),
            Step::Nothing => {
                let step = synonym_texts.iter().flat_map(|(base, _)| {
                    let words = normalize(base);
                    sound_alike_variants(base, &self.sound_alikes)
                        .into_iter()
                        .map(move |v| Variant {
                            text: v,
                            base: Some(words.clone()),
                        })
                });
                match best_of(matcher, step) {
                    Step::Found(hit, _) => (hit, true),
                    Step::Blocked(reason) => return Some(MatchResult::Blocked(reason)),
                    Step::Nothing => return None,
                }
            }
        };
        hit.confidence *= SOUND_ALIKE_FACTOR;
        hit.via_sound_alike = true;
        hit.via_synonym = via_synonym;
        // This candidate reached the threshold before the factor, so it is a
        // real candidate: it ends the search, and the filler step must not
        // decide against it. A text as heard below the threshold is
        // different: the matcher gives `NoCandidate` and does not tell how
        // close the best path came, so the later steps run.
        Some(if hit.confidence >= matcher.threshold() {
            MatchResult::Match(hit)
        } else {
            MatchResult::NoCandidate
        })
    }

    /// Step 5 of `match_text`: the forms of steps 1 to 4 without some of
    /// their filler words.
    ///
    /// - Only words of the text as heard can go. A word that a synonym value
    ///   or a sound-alike replacement gave stays. A filler word in a whole
    ///   name of the vocabulary ("The Old Mill") also stays.
    /// - All variants are one step: a blocked variant, or two variants with
    ///   different tokens, give no match. A form with more filler words than
    ///   the cap (`MAX_FILLER_VARIANTS`) gives no filler step at all.
    /// - A variant that dropped a word of one letter ("a") or a word that
    ///   sounds like a number ("for") gives no match when the dropped word is
    ///   next to a number or a name. "Next to" means the nearest word on
    ///   either side that is not a filler word. A number is a digit or a
    ///   number word. A name is a word that a vocabulary slot of the match
    ///   took. The letter can be a part of a name that the vocabulary does
    ///   not have ("KTL 12099 A"), and "for" can be "four" ("twenty four").
    /// - A dropped "for" is also tried as "four": when that text gives a
    ///   command with another token, or is blocked, the result is no match.
    /// - More than `MAX_FILLER_STEP_VARIANTS` variants over all forms give no
    ///   filler step at all.
    /// - When only one word stays, it must be exact, as in a phrase of one
    ///   word.
    /// - The confidence gets `FILLER_FACTOR` (and `SOUND_ALIKE_FACTOR` for a
    ///   sound-alike form) and must still reach the threshold. The match has
    ///   `via_filler` set.
    fn match_fillers(
        &self,
        matcher: &Matcher,
        text: &str,
        synonym_texts: &[(String, Vec<bool>)],
    ) -> MatchResult {
        if self.fillers.is_empty() {
            return MatchResult::NoCandidate;
        }
        let Some(variants) = self.filler_step_variants(matcher, text, synonym_texts) else {
            return MatchResult::NoCandidate;
        };
        let mut found: Option<Match> = None;
        // The tokens of the number readings ("for" as "4"). Each one must be
        // the token of the result.
        let mut reading_tokens: Vec<String> = Vec::new();
        for variant in variants {
            for reading in &variant.number_readings {
                match match_with_numbers(matcher, reading) {
                    Step::Found(hit, _) => reading_tokens.push(hit.token),
                    Step::Blocked(reason) => return MatchResult::Blocked(reason),
                    Step::Nothing => {}
                }
            }
            let (mut hit, literal) = match match_with_numbers(matcher, &variant.text) {
                Step::Found(hit, literal) => (hit, literal),
                Step::Blocked(reason) => return MatchResult::Blocked(reason),
                Step::Nothing => continue,
            };
            if dropped_next_to_number_or_name(&hit, &variant) {
                return MatchResult::Blocked(BlockReason::DroppedWord);
            }
            let check = Variant {
                text: variant.text.clone(),
                base: variant.sound_alike_base.clone(),
            };
            let one_word_inexact =
                variant.word_count == 1 && (hit.confidence < 1.0 || variant.sound_alike);
            if one_word_inexact || single_required_word_is_inexact(&hit, &check, literal) {
                hit.inexact_single_required_word = true;
            }
            hit.confidence *= FILLER_FACTOR;
            if variant.sound_alike {
                hit.confidence *= SOUND_ALIKE_FACTOR;
            }
            hit.via_filler = true;
            hit.via_synonym = variant.synonym;
            hit.via_sound_alike = variant.sound_alike;
            match &found {
                Some(first) if first.token != hit.token => {
                    return MatchResult::Blocked(BlockReason::Forms)
                }
                Some(first) if first.confidence >= hit.confidence => {}
                _ => found = Some(hit),
            }
        }
        match found {
            Some(hit) if reading_tokens.iter().any(|t| *t != hit.token) => {
                MatchResult::Blocked(BlockReason::Forms)
            }
            Some(hit) if hit.confidence >= matcher.threshold() => MatchResult::Match(hit),
            _ => MatchResult::NoCandidate,
        }
    }

    /// The variants of the filler step, without duplicates. `None` when a
    /// form has more filler words than the cap allows.
    fn filler_step_variants(
        &self,
        matcher: &Matcher,
        text: &str,
        synonym_texts: &[(String, Vec<bool>)],
    ) -> Option<Vec<FillerStepVariant>> {
        // The names with a filler word protect that word.
        let names: Vec<Vec<String>> = matcher
            .names()
            .iter()
            .map(|n| normalize(n))
            .filter(|words| words.iter().any(|w| self.fillers.contains(w)))
            .collect();
        let heard = Form {
            text: text.to_string(),
            spoken: vec![true; text.split_whitespace().count()],
            synonym: false,
            sound_alike_base: None,
        };
        let mut bases = vec![heard];
        bases.extend(synonym_texts.iter().map(|(v, spoken)| Form {
            text: v.clone(),
            spoken: spoken.clone(),
            synonym: true,
            sound_alike_base: None,
        }));
        let mut forms: Vec<Form> = Vec::new();
        for base in &bases {
            let base_tokens: Vec<&str> = base.text.split_whitespace().collect();
            let words = normalize(&base.text);
            for v in sound_alike_variants(&base.text, &self.sound_alikes) {
                // A replaced word is no spoken word: it cannot go.
                let spoken: Vec<bool> = v
                    .split_whitespace()
                    .enumerate()
                    .map(|(i, t)| {
                        base.spoken.get(i).copied().unwrap_or(false)
                            && base_tokens.get(i) == Some(&t)
                    })
                    .collect();
                forms.push(Form {
                    text: v,
                    spoken,
                    synonym: base.synonym,
                    sound_alike_base: Some(words.clone()),
                });
            }
        }
        bases.extend(forms);
        let mut out: Vec<FillerStepVariant> = Vec::new();
        for form in bases {
            let in_names = tokens_in_names(&form.text, &self.fillers, &names);
            let form_words = normalize(&form.text);
            for v in filler_variants_with(&form.text, &self.fillers, &form.spoken, &in_names)? {
                if out.iter().any(|o| o.text == v.text) {
                    continue;
                }
                // The words before the sound-alike replacement, at the kept
                // positions. When the word counts differ, the positions are
                // not clear: the check then gives the cautious answer.
                let sound_alike_base = form.sound_alike_base.as_ref().map(|base| {
                    if base.len() == form_words.len() {
                        v.kept.iter().map(|&k| base[k].clone()).collect()
                    } else {
                        base.clone()
                    }
                });
                out.push(FillerStepVariant {
                    number_readings: v.number_readings.clone(),
                    dropped: v.dropped.clone(),
                    word_count: v.word_count(),
                    text: v.text,
                    synonym: form.synonym,
                    sound_alike: form.sound_alike_base.is_some(),
                    sound_alike_base,
                });
            }
        }
        // A cut-off list could hide a variant with another token.
        (out.len() <= MAX_FILLER_STEP_VARIANTS).then_some(out)
    }

    /// All names that the vocabulary slots can give: the names of the
    /// dynamic vocabulary and the fixed names of the commands.
    pub fn names(&self) -> Vec<String> {
        self.matcher
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .names()
    }

    /// The names and the vocabulary generation of the same vocabulary, read
    /// under one lock.
    pub fn names_and_generation(&self) -> (Vec<String>, u64) {
        let matcher = self.matcher.read().unwrap_or_else(|e| e.into_inner());
        (
            matcher.names(),
            self.vocab_generation.load(Ordering::SeqCst),
        )
    }

    /// The words of all command phrases. A vocabulary change does not
    /// change them.
    pub fn command_words(&self) -> Vec<String> {
        self.matcher
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .command_words()
    }

    /// The number of vocabulary changes since the start. `set_dynamic_vocab`
    /// adds one.
    pub fn vocab_generation(&self) -> u64 {
        self.vocab_generation.load(Ordering::SeqCst)
    }

    /// Returns the delivered match, or `None` (nothing written) when no command matches.
    /// A writer error means the token was not delivered.
    pub fn handle_text(&self, text: &str) -> anyhow::Result<Option<Match>> {
        // The read lock is released before the write: `send` can wait for seconds.
        let Some(m) = self.match_text(text) else {
            return Ok(None);
        };
        self.send_match(&m)?;
        Ok(Some(m))
    }

    /// Sends the token of `hit` without a new match. A writer error means
    /// the token was not delivered.
    pub fn send_match(&self, hit: &Match) -> anyhow::Result<()> {
        self.writer.send(&hit.token)
    }
}

/// The result of the text and its number variants.
enum Step {
    /// A match. The flag is true when the text itself matched, false for a
    /// number variant.
    Found(Match, bool),
    Blocked(BlockReason),
    Nothing,
}

/// The text first. Only when it found no candidate, all number readings of
/// the text: a blocked reading, or two readings with different tokens, give
/// `Blocked`. For one token, the highest confidence stays. A digit-run
/// reading counts only when each joined number is a word of a slot name.
fn match_with_numbers(matcher: &Matcher, text: &str) -> Step {
    match matcher.match_result(text) {
        MatchResult::Match(hit) => return Step::Found(hit, true),
        MatchResult::Blocked(reason) => return Step::Blocked(reason),
        MatchResult::NoCandidate => {}
    }
    let mut found: Option<Match> = None;
    for reading in number_readings(text) {
        let hit = match matcher.match_result(&reading.text) {
            MatchResult::Match(hit) => hit,
            MatchResult::Blocked(reason) => return Step::Blocked(reason),
            MatchResult::NoCandidate => continue,
        };
        // A joined digit run serves only a name: it must land in a slot.
        // Else a doubled word ("five five") could become another value.
        let in_name = reading
            .digit_runs
            .iter()
            .all(|p| hit.slot_positions.contains(p));
        if !in_name {
            continue;
        }
        match &found {
            Some(first) if first.token != hit.token => return Step::Blocked(BlockReason::Forms),
            Some(first) if first.confidence >= hit.confidence => {}
            _ => found = Some(hit),
        }
    }
    match found {
        Some(hit) => Step::Found(hit, false),
        None => Step::Nothing,
    }
}

/// A form of the text before the filler step drops words.
struct Form {
    text: String,
    /// One flag for each whitespace token: a word of the text as heard.
    spoken: Vec<bool>,
    synonym: bool,
    /// For a sound-alike form: the normalised words before the replacement.
    sound_alike_base: Option<Vec<String>>,
}

/// One variant of the filler step.
struct FillerStepVariant {
    text: String,
    /// The dropped words, with their neighbours in the form.
    dropped: Vec<Dropped>,
    /// The number of normalised words of `text`.
    word_count: usize,
    /// The texts with a digit for a dropped word that sounds like a number.
    number_readings: Vec<String>,
    synonym: bool,
    sound_alike: bool,
    /// For a sound-alike form: the words before the replacement, at the kept
    /// positions.
    sound_alike_base: Option<Vec<String>>,
}

/// True when the variant dropped a word of one letter or a word that sounds
/// like a number, and a neighbour of that word is a number or a name.
///
/// A number is a word with a digit or a number word. A name is a word that a
/// vocabulary slot of `hit` took. The slot positions belong to the text that
/// matched: the variant or one of its number readings. All of them count,
/// which is the cautious answer.
fn dropped_next_to_number_or_name(hit: &Match, variant: &FillerStepVariant) -> bool {
    let risky = |word: &str| word.chars().count() == 1 || number_sound_alike(word).is_some();
    if !variant.dropped.iter().any(|d| risky(&d.word)) {
        return false;
    }
    let mut name_words: Vec<String> = Vec::new();
    let texts = std::iter::once(variant.text.clone())
        .chain(number_readings(&variant.text).into_iter().map(|r| r.text));
    for text in texts {
        let words = normalize(&text);
        name_words.extend(
            hit.slot_positions
                .iter()
                .filter_map(|&p| words.get(p).cloned()),
        );
    }
    let number_or_name = |word: &String| {
        word.chars().any(|c| c.is_numeric()) || is_number_word(word) || name_words.contains(word)
    };
    variant
        .dropped
        .iter()
        .filter(|d| risky(&d.word))
        .any(|d| d.neighbours.iter().any(number_or_name))
}

/// A form of the text to try in one step.
struct Variant {
    text: String,
    /// The normalised words before a sound-alike replacement.
    base: Option<Vec<String>>,
}

/// The best match of one step. Each variant is tried with its number
/// variants. A blocked variant, or two variants with different tokens, give
/// `Blocked`. For one token, the highest confidence stays.
fn best_of(matcher: &Matcher, variants: impl Iterator<Item = Variant>) -> Step {
    let mut found: Option<Match> = None;
    for variant in variants {
        let (mut hit, literal) = match match_with_numbers(matcher, &variant.text) {
            Step::Found(hit, literal) => (hit, literal),
            Step::Blocked(reason) => return Step::Blocked(reason),
            Step::Nothing => continue,
        };
        if single_required_word_is_inexact(&hit, &variant, literal) {
            hit.inexact_single_required_word = true;
        }
        match &found {
            Some(first) if first.token != hit.token => return Step::Blocked(BlockReason::Forms),
            Some(first) if first.confidence >= hit.confidence => {}
            _ => found = Some(hit),
        }
    }
    match found {
        Some(hit) => Step::Found(hit, true),
        None => Step::Nothing,
    }
}

/// True when a sound-alike replacement must follow the one-word rule: the
/// replaced word is the only required word, or a slot name of one word.
/// When the positions are not clear (a number variant matched, or the word
/// counts differ), the answer is true for a match with such a word.
fn single_required_word_is_inexact(hit: &Match, variant: &Variant, literal: bool) -> bool {
    let Some(base) = &variant.base else {
        return false;
    };
    let words = normalize(&variant.text);
    let has_single = hit.required_words == 1 || !hit.single_word_slot_positions.is_empty();
    if !literal || words.len() != base.len() {
        return has_single;
    }
    let replaced = |i: &usize| base.get(*i) != words.get(*i);
    (hit.required_words == 1 && hit.required_positions.iter().any(replaced))
        || hit.single_word_slot_positions.iter().any(replaced)
}
