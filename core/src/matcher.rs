//! Deterministic matcher: spoken text -> one command token, or nothing.

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use crate::aliases::AliasMap;
use crate::command_map::{Command, CommandClass, CommandType, PrefixRule};
use crate::grammar::{parse_grammar, GrammarNode};
use crate::profile::BlockingPhrase;

/// Minimum `normalized_levenshtein` similarity for a fuzzy word match.
const WORD_SIMILARITY_FLOOR: f64 = 0.75;

/// Placeholders that take the only captured slot entry when no slot of the same name was captured.
const FALLBACK_PLACEHOLDERS: &[&str] = &["SystemName", "spokenQueuedName"];

/// Scores closer than this count as equal.
const TIE_EPSILON: f64 = 1e-6;

#[derive(Debug, Clone, PartialEq)]
pub struct Match {
    pub token: String,
    /// For a composite command: the lower of the prefix part and the suffix part.
    pub confidence: f32,
    /// A word that must be exact is not exact. The one-word rule of the
    /// session applies to it. It is set in two cases:
    ///
    /// - the required words are one word that is not exact (for a full
    ///   command, the words outside optional groups; for a composite
    ///   command, the name words of the suffix);
    /// - a name from a vocabulary slot is one word that is not exact, or a
    ///   sound-alike replacement (the pipeline sets this case);
    /// - the filler step left one word that is not exact (the pipeline sets
    ///   this case).
    pub inexact_single_required_word: bool,
    /// The match came from a sound-alike variant of the text, not from the
    /// text as heard. `Pipeline::match_text` sets it.
    pub via_sound_alike: bool,
    /// The match came from a synonym variant of the text: the user's own
    /// words for a phrase. `Pipeline::match_text` sets it.
    pub via_synonym: bool,
    /// The match came from a variant of the text without some filler words
    /// ("uh", "the"). `Pipeline::match_text` sets it.
    pub via_filler: bool,
    /// The number of required words: the words outside optional groups, or
    /// the name words of a composite suffix.
    pub required_words: usize,
    /// The positions of the required words in the normalised words of the
    /// matched text.
    pub required_positions: Vec<usize>,
    /// The word similarity of each required word, in the order of
    /// `required_positions`. 1.0 is an exact word.
    pub required_similarities: Vec<f32>,
    /// The positions of the names from vocabulary slots that matched exactly
    /// one word.
    pub single_word_slot_positions: Vec<usize>,
    /// The positions of all words that vocabulary slots matched.
    pub slot_positions: Vec<usize>,
}

/// Why a text gives no command although a candidate reached the threshold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockReason {
    /// Two tokens have the same best confidence.
    Tie,
    /// The best candidate fails its required words or a name of a
    /// vocabulary slot, and no passing path of the same token is better
    /// than every other token.
    RequiredWords,
    /// A path that cannot make its token reached the threshold, and no
    /// token is better by more than `TIE_EPSILON`. See `match_result`.
    NoToken,
    /// Two forms of the text (number readings, synonym, sound-alike or
    /// filler variants) give different tokens, or one of them is blocked.
    /// The pipeline sets it.
    Forms,
    /// The filler step dropped a word of one letter or a word that sounds
    /// like a number next to a number or a name. The pipeline sets it.
    DroppedWord,
}

impl BlockReason {
    /// The name of the reason for the utterance log and the window:
    /// `tie`, `required_words`, `no_token`, `forms` or `dropped_word`.
    pub fn name(self) -> &'static str {
        match self {
            BlockReason::Tie => "tie",
            BlockReason::RequiredWords => "required_words",
            BlockReason::NoToken => "no_token",
            BlockReason::Forms => "forms",
            BlockReason::DroppedWord => "dropped_word",
        }
    }
}

/// The result of one match: a match, a blocked match, or no candidate.
#[derive(Debug, Clone, PartialEq)]
pub enum MatchResult {
    Match(Match),
    /// A candidate reached the threshold, but the program sends nothing.
    /// Another form of the text must not decide it. The causes:
    ///
    /// - two tokens tie (`Tie`);
    /// - the best candidate fails its required words (`RequiredWords`);
    /// - a path that cannot make its token reached the threshold, and no
    ///   token is better by more than the tie margin (`NoToken`). Each
    ///   path counts that `build_token` refuses, not only the phrase of a
    ///   value command without a number variable: a number command whose
    ///   optional number was not spoken, a `{CMD}` template on an alias
    ///   path, captures or command text that are not clear, a composite
    ///   suffix whose prefix rules disagree, and a template with a
    ///   placeholder that no path fills.
    Blocked(BlockReason),
    /// No candidate reached the threshold.
    NoCandidate,
}

impl MatchResult {
    /// The match, or `None` for `Blocked` and `NoCandidate`.
    pub fn into_match(self) -> Option<Match> {
        match self {
            MatchResult::Match(m) => Some(m),
            MatchResult::Blocked(_) | MatchResult::NoCandidate => None,
        }
    }
}

/// Grammar tree with literals already normalised.
enum Node {
    Words {
        words: Vec<String>,
        text: String,
    },
    Alternation(Vec<Node>),
    Optional(Box<Node>),
    Sequence(Vec<Node>),
    Slot(String),
    /// Lowest number, highest number, step.
    Range(u32, u32, u32),
    /// Matches nothing.
    Never,
    /// Matches no words. Marks the end of the prefix part of a composite
    /// command and holds the variables that this prefix sets. `blocked`:
    /// the prefix is the phrase of a left-out command. A composite with it
    /// gives no token.
    PrefixEnd {
        sets: Arc<HashMap<String, String>>,
        blocked: bool,
    },
}

struct VocabEntry {
    words: Vec<String>,
    text: String,
}

struct Grammar {
    root: Node,
    template: String,
    via_alias: bool,
    /// Placeholders that take the number in the spoken phrase.
    number_vars: Vec<String>,
    /// Suffix of a composite command: the conditions on the spoken prefix.
    /// The root then holds only the suffix phrase.
    prefix_rules: Option<Vec<PrefixRule>>,
    /// A value command that cannot fire: its phrase never gives a token.
    /// A text that names it blocks other forms of the text.
    no_token: bool,
}

/// The prefixes and the suffixes of one composite group. The prefixes are
/// walked once per utterance. Each suffix walks on from where a prefix ended.
struct CompositeGroup {
    /// The name of the group.
    name: String,
    /// `prefix phrase, PrefixEnd` sequences: the alternatives of the group.
    prefixes: Vec<Node>,
    suffixes: Vec<Grammar>,
}

/// Best partial path that ends at one word position.
#[derive(Clone, Default)]
struct PathState {
    score: f64,
    /// (slot name, original text of the slot's first entry) in path order.
    captures: Vec<(String, String)>,
    /// Canonical command text pieces for `{CMD}`.
    canonical: Vec<String>,
    /// The text of a slot in `canonical` has a digit, a `.` or a `-`.
    /// VoiceAttack keeps them in its number text, so the path gives no
    /// number token.
    slot_number_text: bool,
    /// Another path with the same score reached this position with different captures.
    ambiguous_captures: bool,
    /// Another path with the same score reached this position with different canonical text.
    ambiguous_canonical: bool,
    /// Composite commands: the number of `canonical` pieces that the prefix gave.
    prefix_end: Option<usize>,
    /// Composite commands: the variables that the matched prefix sets.
    prefix_sets: Option<Arc<HashMap<String, String>>>,
    /// Composite commands: the matched prefix is the phrase of a left-out
    /// command (`PrefixEnd::blocked`). The path gives no token.
    prefix_blocked: bool,
    /// Composite commands: the word position and the score where the prefix ended.
    prefix_pos: usize,
    prefix_score: f64,
    /// How many `Optional` nodes the walk is inside now.
    optional_depth: usize,
    /// The matched words outside every optional group, and the sum of their
    /// similarities. Optional filler words ("look for", "price") do not count.
    /// A composite command counts only its suffix words here: the name, for
    /// example the commodity. `PrefixEnd` sets the count back to zero.
    required_words: usize,
    required_score: f64,
    /// The word positions of the required words.
    required_positions: Vec<usize>,
    /// The similarity of each required word.
    required_similarities: Vec<f64>,
    /// Each name from a vocabulary slot: its first word position and the
    /// similarity of each of its words.
    slots: Vec<(usize, Vec<f64>)>,
}

impl PathState {
    /// Adds the matched words at `pos` to the required words when they are
    /// outside every optional group.
    fn count_required(&mut self, pos: usize, similarities: &[f64]) {
        if self.optional_depth == 0 {
            self.required_words += similarities.len();
            self.required_score += similarities.iter().sum::<f64>();
            self.required_positions
                .extend(pos..pos + similarities.len());
            self.required_similarities.extend_from_slice(similarities);
        }
    }

    /// The average similarity of the required words. `None` without required words.
    fn required_average(&self) -> Option<f64> {
        (self.required_words > 0).then(|| self.required_score / self.required_words as f64)
    }

    /// The required words are one word that is not exact, or a slot name is
    /// one word that is not exact.
    fn inexact_single_required_word(&self) -> bool {
        let slot = self
            .slots
            .iter()
            .any(|(_, sims)| sims.len() == 1 && sims[0] < 1.0 - TIE_EPSILON);
        slot || (self.required_words == 1 && self.required_score < 1.0 - TIE_EPSILON)
    }

    /// Each slot name reaches the threshold on the average of its words, and
    /// each word of it reaches the threshold alone. Thus a weak short word
    /// ("Tou Ling" for "Tou Ming") does not pass on a strong long word.
    fn slots_ok(&self, threshold: f32) -> bool {
        self.slots.iter().all(|(_, sims)| {
            let average = sims.iter().sum::<f64>() / sims.len() as f64;
            average as f32 >= threshold && sims.iter().all(|&s| s as f32 >= threshold)
        })
    }

    /// The positions of all words of the slot names.
    fn slot_positions(&self) -> Vec<usize> {
        self.slots
            .iter()
            .flat_map(|(pos, sims)| *pos..*pos + sims.len())
            .collect()
    }

    /// The positions of the slot names of one word.
    fn single_word_slot_positions(&self) -> Vec<usize> {
        self.slots
            .iter()
            .filter(|(_, sims)| sims.len() == 1)
            .map(|(pos, _)| *pos)
            .collect()
    }
}

/// The result of one grammar path for one token.
#[derive(Clone)]
struct Candidate {
    confidence: f64,
    inexact_single_required_word: bool,
    /// The required words reach the threshold too.
    required_ok: bool,
    required_positions: Vec<usize>,
    required_similarities: Vec<f64>,
    single_word_slot_positions: Vec<usize>,
    slot_positions: Vec<usize>,
}

/// The best candidates of one token: the best of all, and the best that
/// passes its required words.
struct TokenCandidates {
    best: Candidate,
    best_passing: Option<Candidate>,
}

impl TokenCandidates {
    fn add(&mut self, candidate: Candidate) {
        if candidate.required_ok
            && self
                .best_passing
                .as_ref()
                .is_none_or(|b| candidate.confidence > b.confidence)
        {
            self.best_passing = Some(candidate.clone());
        }
        let better_required = (candidate.confidence - self.best.confidence).abs() <= TIE_EPSILON
            && candidate.required_ok
            && !self.best.required_ok;
        if candidate.confidence > self.best.confidence || better_required {
            self.best = candidate;
        }
    }
}

/// Word position -> best path state ending there.
type States = BTreeMap<usize, PathState>;

pub struct Matcher {
    grammars: Vec<Grammar>,
    composites: Vec<CompositeGroup>,
    vocab: HashMap<String, Vec<VocabEntry>>,
    /// The names of the fixed name slots of the commands. A vocabulary
    /// reload does not change them.
    fixed_vocab: HashMap<String, Vec<VocabEntry>>,
    threshold: f32,
    /// The alias keys that no command takes, sorted. Their phrases never
    /// give a token.
    ignored_aliases: Vec<String>,
}

impl Matcher {
    pub fn new(
        commands: Vec<Command>,
        dynamic_vocab: HashMap<String, Vec<String>>,
        aliases: AliasMap,
        threshold: f32,
    ) -> Matcher {
        let mut grammars = Vec::new();
        let mut aliased: HashSet<String> = HashSet::new();
        let fixed_vocab = build_vocab(
            commands
                .iter()
                .flat_map(|c| c.fixed_vocab.clone())
                .collect(),
        );
        // A prefix or a suffix is never a command alone.
        let (parts, commands): (Vec<Command>, Vec<Command>) =
            commands.into_iter().partition(Command::is_composite_part);
        let composites = composite_groups(parts);
        for command in commands {
            // A value command can fire only when the spoken phrase gives its
            // value. Its phrase stays as a grammar without a token: a text
            // that names the command must not become another command.
            if command.class == CommandClass::DynamicValue && command.number_vars.is_empty() {
                if let Some(phrase) = &command.phrase {
                    grammars.push(Grammar {
                        root: compile(&parse_grammar(phrase)),
                        template: command.effective_template(),
                        via_alias: false,
                        number_vars: Vec::new(),
                        prefix_rules: None,
                        no_token: true,
                    });
                }
                continue;
            }
            // Constants are part of the command: build tokens from the
            // effective template, and key aliases by it.
            let template = command.effective_template();
            if let Some(phrase) = &command.phrase {
                grammars.push(Grammar {
                    root: compile(&parse_grammar(phrase)),
                    template: template.clone(),
                    via_alias: false,
                    number_vars: command.number_vars.clone(),
                    prefix_rules: None,
                    no_token: false,
                });
            }
            // Alias phrases belong to the template; add them once.
            if !aliased.insert(template.clone()) {
                continue;
            }
            for phrase in aliases.get(&template).into_iter().flatten() {
                grammars.push(Grammar {
                    root: compile(&parse_grammar(phrase)),
                    template: template.clone(),
                    via_alias: true,
                    number_vars: command.number_vars.clone(),
                    prefix_rules: None,
                    no_token: false,
                });
            }
        }
        // The same decision as above: an alias works only for a template
        // that took aliases, and that an alias path can build.
        let mut ignored_aliases: Vec<String> = aliases
            .keys()
            .filter(|key| !aliased.contains(*key) || needs_spoken_command(key))
            .cloned()
            .collect();
        ignored_aliases.sort();
        let vocab = build_vocab(dynamic_vocab);
        Matcher {
            grammars,
            composites,
            vocab,
            fixed_vocab,
            threshold,
            ignored_aliases,
        }
    }

    /// Adds the phrases of the left-out commands that VoiceAttack
    /// recognises by voice (see [`BlockingPhrase`]) as paths without a
    /// token. A text that names such a command gives `Blocked(NoToken)`
    /// when no token is better by more than the tie margin: a sound-alike,
    /// a synonym, a filler drop or a fuzzy word must not turn it into
    /// another command. A full command gives a grammar. A left-out suffix
    /// joins its composite group as a suffix without a token. A left-out
    /// prefix joins its group as a prefix whose composites give no token.
    /// The working parts and the left-out parts of a group pair with each
    /// other: a group with only working prefixes and left-out suffixes (or
    /// the reverse) blocks the texts of its composites. A group without a
    /// prefix or without a suffix never matches.
    ///
    /// The phrases must have the runtime grammar and the profile variables
    /// in them (see `resolve_blocking`).
    pub fn with_blocking(mut self, phrases: &[BlockingPhrase]) -> Matcher {
        for phrase in phrases {
            let root = compile(&parse_grammar(&phrase.phrase));
            if phrase.command_type == CommandType::Full {
                self.grammars.push(blocked_grammar(root, None));
                continue;
            }
            let name = phrase.composite_group.clone().unwrap_or_default();
            let group = match self.composites.iter().position(|g| g.name == name) {
                Some(k) => &mut self.composites[k],
                None => {
                    self.composites.push(CompositeGroup {
                        name,
                        prefixes: Vec::new(),
                        suffixes: Vec::new(),
                    });
                    self.composites
                        .last_mut()
                        .expect("the group was just added")
                }
            };
            match phrase.command_type {
                CommandType::Suffix => group.suffixes.push(blocked_grammar(root, Some(Vec::new()))),
                CommandType::Prefix => group.prefixes.push(Node::Sequence(vec![
                    root,
                    Node::PrefixEnd {
                        sets: Arc::new(HashMap::new()),
                        blocked: true,
                    },
                ])),
                CommandType::Full => {}
            }
        }
        self
    }

    /// The composite groups with at least one prefix and one suffix. The
    /// other groups never match.
    fn complete_groups(&self) -> impl Iterator<Item = &CompositeGroup> {
        self.composites
            .iter()
            .filter(|g| !g.prefixes.is_empty() && !g.suffixes.is_empty())
    }

    /// The keys of the alias map that no command takes: no command has the
    /// template, the command is a part of a composite group, a value
    /// command has no number variable, the key is the raw template of a
    /// command with a constant, or the template needs the spoken English
    /// command (`{CMD}`). The phrases of these keys never give a token.
    /// The phrases of a `{CMD}` key are in the matcher: a text that they
    /// name blocks (`BlockReason::NoToken`). The phrases of the other keys
    /// are not in the matcher.
    pub fn ignored_alias_keys(&self) -> &[String] {
        &self.ignored_aliases
    }

    /// The minimum confidence of a match.
    pub fn threshold(&self) -> f32 {
        self.threshold
    }

    /// Replaces the vocabulary. The compiled grammars stay.
    pub fn set_dynamic_vocab(&mut self, dynamic_vocab: HashMap<String, Vec<String>>) {
        self.vocab = build_vocab(dynamic_vocab);
    }

    /// The texts of all vocabulary names: the dynamic vocabulary and the
    /// fixed name slots of the commands.
    pub fn names(&self) -> Vec<String> {
        self.vocab
            .values()
            .chain(self.fixed_vocab.values())
            .flatten()
            .map(|e| e.text.clone())
            .collect()
    }

    /// The words of all command phrases (the literal words, not the names
    /// of the slots), without duplicates.
    pub fn command_words(&self) -> Vec<String> {
        let mut words = HashSet::new();
        for grammar in &self.grammars {
            collect_words(&grammar.root, &mut words);
        }
        for group in self.complete_groups() {
            for prefix in &group.prefixes {
                collect_words(prefix, &mut words);
            }
            for suffix in &group.suffixes {
                collect_words(&suffix.root, &mut words);
            }
        }
        let mut words: Vec<String> = words.into_iter().collect();
        words.sort();
        words
    }

    /// The match, or `None`. See `match_result` for the reason of a `None`.
    pub fn match_text(&self, text: &str) -> Option<Match> {
        self.match_result(text).into_match()
    }

    pub fn match_result(&self, text: &str) -> MatchResult {
        let words = normalize(text);
        if words.is_empty() {
            return MatchResult::NoCandidate;
        }
        let mut candidates: BTreeMap<String, TokenCandidates> = BTreeMap::new();
        // The best path that passes its required words but gives no token:
        // each path that `build_token` refuses. For example a command that
        // needs a value that the phrase does not give, or a path whose
        // captures are not clear. See `MatchResult::Blocked`. Also the best
        // such path that fails its required words.
        let mut without_token = WithoutToken::none();
        let start = States::from([(0, PathState::default())]);
        let total = words.len();
        let threshold = self.threshold;
        let mut add = |grammar: &Grammar, end: &States| {
            let Some(state) = end.get(&total) else {
                return;
            };
            let candidate = if grammar.prefix_rules.is_some() {
                // Composite: each part must pass on its own words. The suffix
                // counts only its name words, not the optional filler words.
                // A prefix without words or a suffix without a name never fires.
                let Some(suffix) = state.required_average() else {
                    return;
                };
                if state.prefix_pos == 0 {
                    return;
                }
                let prefix = state.prefix_score / state.prefix_pos as f64;
                Candidate {
                    confidence: prefix.min(suffix),
                    inexact_single_required_word: state.inexact_single_required_word(),
                    required_ok: state.slots_ok(threshold),
                    required_positions: state.required_positions.clone(),
                    required_similarities: state.required_similarities.clone(),
                    single_word_slot_positions: state.single_word_slot_positions(),
                    slot_positions: state.slot_positions(),
                }
            } else {
                // Full command: the words outside optional groups must also
                // pass the threshold alone. A phrase of only optional words
                // has no extra condition. Each slot name must pass alone too.
                let required_ok = state
                    .required_average()
                    .is_none_or(|score| score as f32 >= threshold)
                    && state.slots_ok(threshold);
                Candidate {
                    confidence: state.score / total as f64,
                    inexact_single_required_word: state.inexact_single_required_word(),
                    required_ok,
                    required_positions: state.required_positions.clone(),
                    required_similarities: state.required_similarities.clone(),
                    single_word_slot_positions: state.single_word_slot_positions(),
                    slot_positions: state.slot_positions(),
                }
            };
            let Some(token) = build_token(grammar, state) else {
                if candidate.required_ok {
                    without_token.passing = without_token.passing.max(candidate.confidence);
                }
                without_token.any = without_token.any.max(candidate.confidence);
                return;
            };
            match candidates.entry(token) {
                Entry::Vacant(slot) => {
                    let best_passing = candidate.required_ok.then(|| candidate.clone());
                    slot.insert(TokenCandidates {
                        best: candidate,
                        best_passing,
                    });
                }
                Entry::Occupied(mut slot) => slot.get_mut().add(candidate),
            }
        };
        for grammar in &self.grammars {
            add(grammar, &self.walk(&grammar.root, &start, &words));
        }
        for group in self.complete_groups() {
            let mut after_prefix = States::new();
            for prefix in &group.prefixes {
                for (pos, state) in self.walk(prefix, &start, &words) {
                    merge(&mut after_prefix, pos, state);
                }
            }
            // A prefix that matched no words is no prefix.
            after_prefix.remove(&0);
            if after_prefix.is_empty() {
                continue;
            }
            for suffix in &group.suffixes {
                add(suffix, &self.walk(&suffix.root, &after_prefix, &words));
            }
        }

        decide(&candidates, without_token, self.threshold)
    }

    /// Advances every input state through `node`.
    fn walk(&self, node: &Node, input: &States, words: &[String]) -> States {
        let mut out = States::new();
        match node {
            Node::Words {
                words: pattern,
                text,
            } => {
                for (&pos, state) in input {
                    if let Some(score) = match_words(pattern, words, pos) {
                        let mut next = state.clone();
                        next.score += score;
                        if !pattern.is_empty() {
                            next.canonical.push(text.clone());
                            next.count_required(pos, &similarities(pattern, words, pos));
                        }
                        merge(&mut out, pos + pattern.len(), next);
                    }
                }
            }
            Node::Never => {}
            Node::PrefixEnd { sets, blocked } => {
                for (&pos, state) in input {
                    let mut next = state.clone();
                    next.prefix_end = Some(next.canonical.len());
                    next.prefix_sets = Some(Arc::clone(sets));
                    next.prefix_blocked = *blocked;
                    next.prefix_pos = pos;
                    next.prefix_score = next.score;
                    // Only the suffix words are required words of a composite.
                    next.required_words = 0;
                    next.required_score = 0.0;
                    next.required_positions.clear();
                    next.required_similarities.clear();
                    merge(&mut out, pos, next);
                }
            }
            Node::Slot(name) => {
                let entries = self
                    .vocab
                    .get(name)
                    .or_else(|| self.fixed_vocab.get(name))
                    .map(Vec::as_slice)
                    .unwrap_or(&[]);
                // The first entry is the canonical name; the others are alternates
                // (phonetic spellings, stations in the system).
                let Some(first) = entries.first() else {
                    return out;
                };
                for (&pos, state) in input {
                    for entry in entries {
                        if let Some(score) = match_words(&entry.words, words, pos) {
                            let mut next = state.clone();
                            next.score += score;
                            next.captures.push((name.clone(), first.text.clone()));
                            next.canonical.push(entry.text.clone());
                            next.slot_number_text |= entry.text.contains(kept_in_number_text);
                            let sims = similarities(&entry.words, words, pos);
                            next.count_required(pos, &sims);
                            next.slots.push((pos, sims));
                            merge(&mut out, pos + entry.words.len(), next);
                        }
                    }
                }
            }
            Node::Range(lo, hi, step) => {
                for (&pos, state) in input {
                    let Some(word) = words.get(pos) else {
                        continue;
                    };
                    if !word.chars().all(|c| c.is_ascii_digit()) {
                        continue;
                    }
                    // "05" is no value: a leading zero would give a new
                    // number text that the phrase never had.
                    if word.len() > 1 && word.starts_with('0') {
                        continue;
                    }
                    let in_range =
                        |n: u32| (*lo..=*hi).contains(&n) && (n - *lo).is_multiple_of(*step);
                    if word.parse::<u32>().is_ok_and(in_range) {
                        let mut next = state.clone();
                        next.score += 1.0;
                        next.canonical.push(word.clone());
                        next.count_required(pos, &[1.0]);
                        merge(&mut out, pos + 1, next);
                    }
                }
            }
            Node::Optional(inner) => {
                out = input.clone();
                // Words inside an optional group are filler, not name words.
                let inside: States = input
                    .iter()
                    .map(|(&pos, state)| {
                        let mut state = state.clone();
                        state.optional_depth += 1;
                        (pos, state)
                    })
                    .collect();
                for (pos, mut state) in self.walk(inner, &inside, words) {
                    state.optional_depth -= 1;
                    merge(&mut out, pos, state);
                }
            }
            Node::Alternation(alternatives) => {
                for alternative in alternatives {
                    for (pos, state) in self.walk(alternative, input, words) {
                        merge(&mut out, pos, state);
                    }
                }
            }
            Node::Sequence(items) => {
                out = input.clone();
                for item in items {
                    if out.is_empty() {
                        break;
                    }
                    out = self.walk(item, &out, words);
                }
            }
        }
        out
    }
}

/// The best confidence of the paths without a token (`f64::NEG_INFINITY`
/// when there is none).
#[derive(Debug, Clone, Copy)]
struct WithoutToken {
    /// The best path that passes its required words.
    passing: f64,
    /// The best path, also one that fails its required words.
    any: f64,
}

impl WithoutToken {
    fn none() -> WithoutToken {
        WithoutToken {
            passing: f64::NEG_INFINITY,
            any: f64::NEG_INFINITY,
        }
    }
}

/// The result of one text from the best candidates of each token and the
/// best paths without a token.
fn decide(
    candidates: &BTreeMap<String, TokenCandidates>,
    without_token: WithoutToken,
    threshold: f32,
) -> MatchResult {
    let best_without_token = without_token.passing;
    // A path without a token that reaches the threshold is the command
    // that the text names. It blocks when no other token is clearly
    // better: a lower candidate, a sound-alike or a synonym form of the
    // text must not turn it into another command. The safe direction:
    // also a path that only this text cannot make (for example a number
    // command without its number) blocks.
    let unbuilt = best_without_token as f32 >= threshold;
    let miss = if unbuilt {
        MatchResult::Blocked(BlockReason::NoToken)
    } else {
        MatchResult::NoCandidate
    };
    let Some((token, top)) = candidates
        .iter()
        .max_by(|a, b| a.1.best.confidence.total_cmp(&b.1.best.confidence))
    else {
        return miss;
    };
    let confidence = top.best.confidence;
    if (confidence as f32) < threshold {
        return miss;
    }
    if unbuilt && confidence - best_without_token <= TIE_EPSILON {
        return MatchResult::Blocked(BlockReason::NoToken);
    }
    // The best of each other token, passing or not.
    let other_best = candidates
        .iter()
        .filter(|(t, _)| *t != token)
        .map(|(_, c)| c.best.confidence)
        .fold(f64::NEG_INFINITY, f64::max);
    if (confidence - other_best).abs() <= TIE_EPSILON {
        return MatchResult::Blocked(BlockReason::Tie);
    }
    // The best path of the token fails its required words. A path of
    // the same token that passes them can still win, but only when it
    // is better than every path of every other token.
    let chosen = if top.best.required_ok {
        &top.best
    } else {
        match &top.best_passing {
            Some(passing)
                if passing.confidence as f32 >= threshold
                    && passing.confidence > other_best + TIE_EPSILON =>
            {
                passing
            }
            _ => return MatchResult::Blocked(BlockReason::RequiredWords),
        }
    };
    // The margin applies to the path that is sent, not to the best
    // path of the token. A path without a token blocks here also when it
    // fails its required words: the text names that command at least as
    // well as the path that would be sent.
    if chosen.confidence - without_token.any <= TIE_EPSILON {
        return MatchResult::Blocked(BlockReason::NoToken);
    }
    MatchResult::Match(Match {
        token: token.clone(),
        confidence: chosen.confidence as f32,
        inexact_single_required_word: chosen.inexact_single_required_word,
        via_sound_alike: false,
        via_synonym: false,
        via_filler: false,
        required_words: chosen.required_positions.len(),
        required_positions: chosen.required_positions.clone(),
        required_similarities: chosen
            .required_similarities
            .iter()
            .map(|&s| s as f32)
            .collect(),
        single_word_slot_positions: chosen.single_word_slot_positions.clone(),
        slot_positions: chosen.slot_positions.clone(),
    })
}

/// Normalises every entry. The first entry of a slot stays the canonical name.
fn build_vocab(dynamic_vocab: HashMap<String, Vec<String>>) -> HashMap<String, Vec<VocabEntry>> {
    dynamic_vocab
        .into_iter()
        .map(|(name, entries)| {
            let entries = entries
                .into_iter()
                .map(|text| VocabEntry {
                    words: normalize(&text),
                    text,
                })
                .filter(|e| !e.words.is_empty())
                .collect();
            (name, entries)
        })
        .collect()
}

/// A grammar without a token for the phrase of a left-out command.
/// `prefix_rules`: `Some` for a suffix of a composite group.
fn blocked_grammar(root: Node, prefix_rules: Option<Vec<PrefixRule>>) -> Grammar {
    Grammar {
        root,
        template: String::new(),
        via_alias: false,
        number_vars: Vec::new(),
        prefix_rules,
        no_token: true,
    }
}

/// The composite groups of the working parts, in the order of the group
/// names. A group can have only prefixes or only suffixes: the left-out
/// parts of `Matcher::with_blocking` can complete it. A part without a
/// phrase or a group gives nothing.
fn composite_groups(parts: Vec<Command>) -> Vec<CompositeGroup> {
    let mut groups: BTreeMap<String, CompositeGroup> = BTreeMap::new();
    for part in parts {
        let (Some(phrase), Some(name)) = (part.phrase.as_deref(), part.composite_group.clone())
        else {
            continue;
        };
        let root = compile(&parse_grammar(phrase));
        let group = groups
            .entry(name.clone())
            .or_insert_with(|| CompositeGroup {
                name,
                prefixes: Vec::new(),
                suffixes: Vec::new(),
            });
        match part.command_type {
            CommandType::Prefix => group.prefixes.push(Node::Sequence(vec![
                root,
                Node::PrefixEnd {
                    sets: Arc::new(part.sets.clone()),
                    blocked: false,
                },
            ])),
            CommandType::Suffix => group.suffixes.push(Grammar {
                root,
                template: part.effective_template(),
                via_alias: false,
                number_vars: part.number_vars.clone(),
                prefix_rules: Some(part.prefix_rules.clone()),
                no_token: false,
            }),
            CommandType::Full => {}
        }
    }
    groups.into_values().collect()
}

/// The variables of a composite command, by lowercase name: the prefix `sets`,
/// then the variables of each suffix rule whose text the spoken prefix
/// contains (case-insensitive). `None` when two matching rules give one
/// variable different values: the prefix is not clear.
fn composite_vars(rules: &[PrefixRule], state: &PathState) -> Option<HashMap<String, String>> {
    let end = state.prefix_end?;
    let prefix = state.canonical.get(..end)?.join(" ").to_lowercase();
    let mut vars: HashMap<String, String> = state
        .prefix_sets
        .as_deref()?
        .iter()
        .map(|(name, value)| (name.to_lowercase(), value.clone()))
        .collect();
    let mut from_rules: HashMap<String, String> = HashMap::new();
    for rule in rules {
        if !prefix.contains(&rule.contains.to_lowercase()) {
            continue;
        }
        for (name, value) in &rule.set {
            let name = name.to_lowercase();
            if from_rules.get(&name).is_some_and(|v| v != value) {
                return None;
            }
            from_rules.insert(name, value.clone());
        }
    }
    vars.extend(from_rules);
    Some(vars)
}

/// Keeps the best state per position and marks equal-score conflicts.
fn merge(states: &mut States, pos: usize, new: PathState) {
    match states.entry(pos) {
        Entry::Vacant(slot) => {
            slot.insert(new);
        }
        Entry::Occupied(mut slot) => {
            let current = slot.get_mut();
            if new.score > current.score + TIE_EPSILON {
                *current = new;
            } else if (new.score - current.score).abs() <= TIE_EPSILON {
                current.ambiguous_captures |=
                    new.ambiguous_captures || new.captures != current.captures;
                // A different prefix end splits the same words into a
                // different prefix text, so it counts as different text too.
                current.ambiguous_canonical |= new.ambiguous_canonical
                    || new.canonical != current.canonical
                    || new.prefix_end != current.prefix_end
                    || new.prefix_sets != current.prefix_sets;
                current.slot_number_text |= new.slot_number_text;
                // A left-out prefix with the same score: the safe side.
                current.prefix_blocked |= new.prefix_blocked;
            }
        }
    }
}

/// Sum of word similarities when `pattern` matches the spoken words at `pos`.
fn match_words(pattern: &[String], words: &[String], pos: usize) -> Option<f64> {
    let spoken = words.get(pos..pos + pattern.len())?;
    pattern
        .iter()
        .zip(spoken)
        .map(|(a, b)| word_similarity(a, b))
        .sum()
}

/// The similarity of each word of `pattern` to the spoken words at `pos`.
/// Call it only after `match_words` gave a score: each word then matches.
fn similarities(pattern: &[String], words: &[String], pos: usize) -> Vec<f64> {
    pattern
        .iter()
        .zip(&words[pos..])
        .map(|(a, b)| word_similarity(a, b).unwrap_or(0.0))
        .collect()
}

/// Adds the literal words of the node to `out`.
fn collect_words(node: &Node, out: &mut HashSet<String>) {
    match node {
        Node::Words { words, .. } => out.extend(words.iter().cloned()),
        Node::Alternation(nodes) | Node::Sequence(nodes) => {
            for n in nodes {
                collect_words(n, out);
            }
        }
        Node::Optional(inner) => collect_words(inner, out),
        Node::Slot(_) | Node::Range(..) | Node::Never | Node::PrefixEnd { .. } => {}
    }
}

/// The similarity of two different normalised words when the matcher takes
/// one as an inexact form of the other. `None` for equal words and for words
/// that the matcher does not take as a fuzzy match.
pub fn inexact_word_similarity(a: &str, b: &str) -> Option<f64> {
    if a == b {
        return None;
    }
    word_similarity(a, b)
}

fn word_similarity(a: &str, b: &str) -> Option<f64> {
    if a == b {
        return Some(1.0);
    }
    let has_digit = |s: &str| s.chars().any(char::is_numeric);
    if has_digit(a) || has_digit(b) {
        return None;
    }
    let similarity = strsim::normalized_levenshtein(a, b);
    (similarity >= WORD_SIMILARITY_FLOOR).then_some(similarity)
}

/// Lowercase, drop apostrophes, turn other non-alphanumerics into spaces, split on whitespace.
pub fn normalize(text: &str) -> Vec<String> {
    let cleaned: String = text
        .to_lowercase()
        .chars()
        .filter(|c| !matches!(c, '\'' | '\u{2018}' | '\u{2019}'))
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    cleaned.split_whitespace().map(str::to_string).collect()
}

fn compile(node: &GrammarNode) -> Node {
    match node {
        // Range syntax that the parser could not read, for example `-10..10`.
        GrammarNode::Literal(text) if text.contains("..") => Node::Never,
        GrammarNode::Literal(text) => Node::Words {
            words: normalize(text),
            text: text.clone(),
        },
        GrammarNode::Alternation(nodes) => Node::Alternation(nodes.iter().map(compile).collect()),
        GrammarNode::Optional(inner) => Node::Optional(Box::new(compile(inner))),
        GrammarNode::Sequence(nodes) => Node::Sequence(nodes.iter().map(compile).collect()),
        GrammarNode::ListSlot(name) => Node::Slot(name.clone()),
        GrammarNode::Never => Node::Never,
        GrammarNode::Range(lo, hi) => Node::Range(*lo, *hi, 1),
        GrammarNode::ScaledRange(lo, hi, multiplier) => {
            match (lo.checked_mul(*multiplier), hi.checked_mul(*multiplier)) {
                (Some(lo), Some(hi)) => Node::Range(lo, hi, *multiplier),
                // The numbers do not fit in a u32.
                _ => Node::Never,
            }
        }
    }
}

/// Replaces the template placeholders. `None` when any placeholder stays unresolved.
fn build_token(grammar: &Grammar, state: &PathState) -> Option<String> {
    if grammar.no_token || state.prefix_blocked {
        return None;
    }
    let template = grammar.template.as_str();
    if state.ambiguous_captures && template.contains("{TXT:") {
        return None;
    }
    // A number placeholder comes from the canonical text, as `{CMD}` does.
    let uses_canonical = template.contains("{CMD}") || !grammar.number_vars.is_empty();
    if state.ambiguous_canonical && uses_canonical {
        return None;
    }
    // A composite command reads the spoken prefix text.
    let vars = match &grammar.prefix_rules {
        Some(_) if state.ambiguous_canonical => return None,
        Some(rules) => Some(composite_vars(rules, state)?),
        None => None,
    };
    let mut token = String::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        // An unclosed `{` would copy placeholder text into the token.
        let close = open + rest[open..].find('}')?;
        token.push_str(&rest[..open]);
        token.push_str(&resolve_placeholder(
            &rest[open + 1..close],
            grammar,
            state,
            vars.as_ref(),
        )?);
        rest = &rest[close + 1..];
    }
    token.push_str(rest);
    Some(token)
}

/// A character that VoiceAttack keeps in the number text of `{TXTNUM:}`.
fn kept_in_number_text(c: char) -> bool {
    c == '.' || c == '-' || c.is_numeric()
}

/// The digits of the canonical command text, joined. `None` when it has none.
///
/// VoiceAttack's `{TXTNUM:}` also keeps `.` and `-`. The builder and the
/// runtime grammar check leave out a number command whose phrase has them
/// outside its ranges, except a `-` with a twin spelling (see
/// `number_phrase`). A slot text with them gives no token (see
/// `resolve_placeholder`). Thus they are dropped here.
fn spoken_number(canonical: &[String]) -> Option<String> {
    let digits: String = canonical
        .iter()
        .flat_map(|piece| piece.chars())
        .filter(char::is_ascii_digit)
        .collect();
    (!digits.is_empty()).then_some(digits)
}

/// True when the token template has `{CMD}`. Such a token needs the
/// English command text, so an alias path cannot build it (see
/// `resolve_placeholder`).
pub fn needs_spoken_command(template: &str) -> bool {
    template.contains("{CMD}")
}

/// `vars`: the variables of a composite command, by lowercase name.
fn resolve_placeholder(
    name: &str,
    grammar: &Grammar,
    state: &PathState,
    vars: Option<&HashMap<String, String>>,
) -> Option<String> {
    if name == "CMD" {
        // EDCoPilot expects the English command text, which an alias path does not give.
        return (!grammar.via_alias).then(|| state.canonical.join(" "));
    }
    let slot = name.strip_prefix("TXT:")?;
    // VoiceAttack variable names are not case-sensitive.
    if let Some(value) = vars.and_then(|v| v.get(&slot.to_lowercase())) {
        return Some(value.clone());
    }
    if grammar.number_vars.iter().any(|v| v == slot) {
        if state.slot_number_text {
            return None;
        }
        return spoken_number(&state.canonical);
    }
    let mut by_name = state
        .captures
        .iter()
        .filter(|(captured, _)| captured == slot)
        .map(|(_, text)| text);
    if let Some(first) = by_name.next() {
        // The same slot captured with different texts is unresolved.
        return by_name.all(|t| t == first).then(|| first.clone());
    }
    match state.captures.as_slice() {
        [(_, text)] if FALLBACK_PLACEHOLDERS.contains(&slot) => Some(text.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command_map::CommandClass;

    const SET_DEST: &str = r#"SailTo:"{TXT:SystemName}""#;

    fn cmd(phrase: &str, template: &str, class: CommandClass) -> Command {
        Command {
            phrase: Some(phrase.to_string()),
            token_template: template.to_string(),
            class,
            var_source: None,
            category: None,
            constants: HashMap::new(),
            number_vars: Vec::new(),
            ..Default::default()
        }
    }

    /// A value command whose placeholder takes the number in the spoken phrase.
    fn number_cmd(phrase: &str, template: &str, vars: &[&str]) -> Command {
        let mut command = cmd(phrase, template, CommandClass::DynamicValue);
        command.number_vars = vars.iter().map(|v| v.to_string()).collect();
        command
    }

    fn number_matcher(command: Command) -> Matcher {
        Matcher::new(vec![command], HashMap::new(), AliasMap::new(), 0.8)
    }

    #[test]
    fn test_number_var_takes_the_spoken_number() {
        let m = number_matcher(number_cmd(
            "[buoys from; go to] [6..30] thousand sea miles",
            "ShowBuoysFromDistance={TXT:MilesVar}000",
            &["MilesVar"],
        ));
        assert_eq!(
            token(&m, "go to 12 thousand sea miles").as_deref(),
            Some("ShowBuoysFromDistance=12000")
        );
        assert_eq!(token(&m, "go to 31 thousand sea miles"), None);
    }

    #[test]
    fn test_number_var_joins_all_numbers_of_the_phrase() {
        let m = number_matcher(number_cmd(
            "[button; pick; slot] [1..6] [1..6]",
            "PickMooringVoice:{TXT:MooringCoord}",
            &["MooringCoord"],
        ));
        assert_eq!(
            token(&m, "pick 3 4").as_deref(),
            Some("PickMooringVoice:34")
        );
    }

    #[test]
    fn test_scaled_range_matches_only_its_steps() {
        let m = number_matcher(number_cmd(
            "[buoys from; go to] [3..49,10] sea miles",
            "ShowBuoysFromDistance={TXT:MilesVar}",
            &["MilesVar"],
        ));
        assert_eq!(
            token(&m, "go to 250 sea miles").as_deref(),
            Some("ShowBuoysFromDistance=250")
        );
        assert_eq!(
            token(&m, "go to 30 sea miles").as_deref(),
            Some("ShowBuoysFromDistance=30")
        );
        assert_eq!(
            token(&m, "go to 490 sea miles").as_deref(),
            Some("ShowBuoysFromDistance=490")
        );
        // Not a step, below the range, above the range.
        assert_eq!(token(&m, "go to 255 sea miles"), None);
        assert_eq!(token(&m, "go to 20 sea miles"), None);
        assert_eq!(token(&m, "go to 500 sea miles"), None);
    }

    #[test]
    fn test_number_from_literal_alternatives() {
        let m = number_matcher(number_cmd(
            "launch dinghy [1;2]",
            "launchDinghyNum={TXT:crewDinghyNum}",
            &["crewDinghyNum"],
        ));
        assert_eq!(
            token(&m, "launch dinghy 2").as_deref(),
            Some("launchDinghyNum=2")
        );
        assert_eq!(token(&m, "launch dinghy 3"), None);
    }

    /// A literal digit of the phrase goes into the number in phrase order,
    /// also before or after a range. VoiceAttack keeps each digit of the
    /// command text in the same order.
    #[test]
    fn test_number_joins_literal_digits_in_phrase_order() {
        let cases = [
            ("berth [0;] [1..9]", "berth 0 5", Some("05")),
            ("berth [0;] [1..9]", "berth 5", Some("5")),
            ("[1..9] [0;] lamps", "5 0 lamps", Some("50")),
            ("[1..9] [0;] lamps", "5 lamps", Some("5")),
            ("dock2 lamp [1..9]", "dock2 lamp 4", Some("24")),
            ("lamp [1..9] row 7", "lamp 4 row 7", Some("47")),
        ];
        for (phrase, text, expected) in cases {
            let m = number_matcher(number_cmd(phrase, "Lamp={TXT:Row}", &["Row"]));
            let expected = expected.map(|n| format!("Lamp={n}"));
            assert_eq!(token(&m, text), expected, "{phrase}: {text}");
        }
    }

    /// The number is digits only. A hyphen in the phrase text is not a sign.
    #[test]
    fn test_number_ignores_hyphen_in_phrase_text() {
        let m = number_matcher(number_cmd(
            "[copilot; co-pilot] horn rate [10..30, 10]",
            "Prefs:HornRate=\"{TXT:HornNumber}\"",
            &["HornNumber"],
        ));
        assert_eq!(
            token(&m, "co-pilot horn rate 150").as_deref(),
            Some("Prefs:HornRate=\"150\"")
        );
    }

    /// VoiceAttack takes the number from the whole command text, also from
    /// the text of a slot. A slot name with a digit, a point or a minus
    /// gives no number token.
    #[test]
    fn test_number_var_with_number_text_in_a_slot_does_not_fire() {
        let vocab = HashMap::from([
            ("System4".to_string(), vec!["Pier 3081".to_string()]),
            ("System5".to_string(), vec!["Ness-Wick".to_string()]),
            ("System6".to_string(), vec!["Morrow".to_string()]),
        ]);
        let m = Matcher::new(
            ["System4", "System5", "System6"]
                .iter()
                .map(|s| {
                    number_cmd(
                        &format!("moor at {{TXT:{s}}} berth [1..9]"),
                        "Moor={TXT:Berth}",
                        &["Berth"],
                    )
                })
                .collect(),
            vocab,
            AliasMap::new(),
            0.8,
        );
        assert_eq!(
            token(&m, "moor at morrow berth 5").as_deref(),
            Some("Moor=5")
        );
        assert_eq!(token(&m, "moor at pier 3081 berth 5"), None);
        assert_eq!(token(&m, "moor at ness wick berth 5"), None);
    }

    #[test]
    fn test_number_var_without_a_spoken_number_does_not_fire() {
        let m = number_matcher(number_cmd(
            "bell volume [1..20, 5;]",
            "Prefs:BellVolume=\"{TXT:BellNumber}\"",
            &["BellNumber"],
        ));
        assert_eq!(token(&m, "bell volume"), None);
        assert_eq!(
            token(&m, "bell volume 55").as_deref(),
            Some("Prefs:BellVolume=\"55\"")
        );
    }

    #[test]
    fn test_number_var_with_another_unresolved_placeholder_does_not_fire() {
        let m = number_matcher(number_cmd(
            "move crate [1..30] to the [top; bottom]",
            "MoveCrate={TXT:crateCoord}-{TXT:cratePlacement}",
            &["crateCoord"],
        ));
        assert_eq!(token(&m, "move crate 3 to the top"), None);
    }

    /// The parser keeps a range it cannot read (negative numbers) as literal
    /// text. Its digits must not match as plain words and become a value.
    #[test]
    fn test_unreadable_range_never_matches() {
        let m = number_matcher(number_cmd(
            "[bell;] tone [-10..10]",
            "Tune:BellTone=\"{TXT:toneStep}\"",
            &["toneStep"],
        ));
        assert_eq!(token(&m, "tone 10 10"), None);
        assert_eq!(token(&m, "bell tone -10..10"), None);
        assert_eq!(token(&m, "tone 5"), None);
    }

    #[test]
    fn test_number_var_through_an_alias() {
        let command = number_cmd(
            "launch dinghy [1;2]",
            "launchDinghyNum={TXT:crewDinghyNum}",
            &["crewDinghyNum"],
        );
        let aliases = AliasMap::from([(
            command.token_template.clone(),
            vec!["beiboot [1..2] starten".to_string()],
        )]);
        let m = Matcher::new(vec![command], HashMap::new(), aliases, 0.8);
        assert_eq!(
            token(&m, "Beiboot 2 starten").as_deref(),
            Some("launchDinghyNum=2")
        );
    }

    fn with_system(mut command: Command, system: &str) -> Command {
        command
            .constants
            .insert("SystemName".to_string(), system.to_string());
        command
    }

    const SPEC_COURSE: &str = "HarbourInfo:{TXT:SystemName}:sail out";

    #[test]
    fn test_constant_fills_placeholder() {
        let graymouth = with_system(
            cmd(
                "[sail out for; sail out to] Graymouth",
                SPEC_COURSE,
                CommandClass::DynamicName,
            ),
            "Dunmore",
        );
        let m = Matcher::new(vec![graymouth], HashMap::new(), AliasMap::new(), 0.8);
        assert_eq!(
            token(&m, "sail out to graymouth").as_deref(),
            Some("HarbourInfo:Dunmore:sail out")
        );
    }

    #[test]
    fn test_constant_together_with_cmd() {
        let keeper = with_system(
            cmd(
                "[depth at; berths in] Marta Quillon",
                "HarbourInfo:{TXT:SystemName}:{CMD}",
                CommandClass::DynamicName,
            ),
            "Kessmoor",
        );
        let m = Matcher::new(vec![keeper], HashMap::new(), AliasMap::new(), 0.8);
        assert_eq!(
            token(&m, "berths in marta quillon").as_deref(),
            Some("HarbourInfo:Kessmoor:berths in Marta Quillon")
        );
    }

    /// Commands that share a raw template but write different constants are
    /// different commands; an alias must reach only the one it names.
    #[test]
    fn test_alias_is_keyed_by_effective_template() {
        let graymouth = with_system(
            cmd(
                "[sail out for; sail out to] Graymouth",
                SPEC_COURSE,
                CommandClass::DynamicName,
            ),
            "Dunmore",
        );
        let plotted = cmd(
            "[sail out for; sail out to] {TXT:System1}",
            SPEC_COURSE,
            CommandClass::DynamicName,
        );
        let aliases = AliasMap::from([
            (
                "HarbourInfo:Dunmore:sail out".to_string(),
                vec!["kurs nach graymouth".to_string()],
            ),
            (
                SPEC_COURSE.to_string(),
                vec!["kurs setzen nach {TXT:System1}".to_string()],
            ),
        ]);
        let m = Matcher::new(
            vec![graymouth, plotted],
            vocab(&[("System1", &["Ceos"])]),
            aliases,
            0.8,
        );
        assert_eq!(
            token(&m, "kurs nach graymouth").as_deref(),
            Some("HarbourInfo:Dunmore:sail out")
        );
        assert_eq!(
            token(&m, "kurs setzen nach ceos").as_deref(),
            Some("HarbourInfo:Ceos:sail out")
        );
    }

    fn vocab(pairs: &[(&str, &[&str])]) -> HashMap<String, Vec<String>> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.iter().map(|s| s.to_string()).collect()))
            .collect()
    }

    fn matcher(commands: Vec<Command>, vocab: HashMap<String, Vec<String>>) -> Matcher {
        Matcher::new(commands, vocab, AliasMap::new(), 0.8)
    }

    fn voice_log() -> Command {
        cmd(
            "[open; show;] [the;] chart log [panel;]",
            "OpenChartLogPanel",
            CommandClass::Fixed,
        )
    }

    fn token(m: &Matcher, text: &str) -> Option<String> {
        m.match_text(text).map(|m| m.token)
    }

    #[test]
    fn test_fixed_exact() {
        let m = matcher(vec![voice_log()], HashMap::new());
        let hit = m.match_text("open chart log").unwrap();
        assert_eq!(hit.token, "OpenChartLogPanel");
        assert_eq!(hit.confidence, 1.0);
    }

    #[test]
    fn test_dynamic_name_substitution() {
        let m = matcher(
            vec![cmd(
                "sail out to <System>",
                SET_DEST,
                CommandClass::DynamicName,
            )],
            vocab(&[("System", &["Ceos"])]),
        );
        assert_eq!(
            token(&m, "sail out to Ceos").as_deref(),
            Some("SailTo:\"Ceos\"")
        );
    }

    /// A slot of another kind than a system slot fills its token with the
    /// first name of the slot.
    #[test]
    fn test_a_voice_slot_fills_the_token() {
        let m = matcher(
            vec![cmd(
                "speak as {TXT:Voice2}",
                "Voice:{TXT:Voice2}",
                CommandClass::DynamicName,
            )],
            vocab(&[("Voice2", &["Harbour Crier"])]),
        );
        assert_eq!(
            token(&m, "speak as harbour crier").as_deref(),
            Some("Voice:Harbour Crier")
        );
        assert_eq!(token(&m, "speak as harbour"), None);
    }

    #[test]
    fn test_name_with_space() {
        let m = matcher(
            vec![cmd(
                "sail out to <System>",
                SET_DEST,
                CommandClass::DynamicName,
            )],
            vocab(&[("System", &["TRV 48213"])]),
        );
        assert_eq!(
            token(&m, "sail out to TRV 48213").as_deref(),
            Some("SailTo:\"TRV 48213\"")
        );
    }

    #[test]
    fn test_real_shape_system_specific() {
        let m = matcher(
            vec![cmd(
                "[sail out for; sail out to] {TXT:System96}",
                "HarbourInfo:{TXT:SystemName}:sail out",
                CommandClass::DynamicName,
            )],
            vocab(&[("System96", &["Ceos"])]),
        );
        assert_eq!(
            token(&m, "sail out to ceos").as_deref(),
            Some("HarbourInfo:Ceos:sail out")
        );
    }

    #[test]
    fn test_ambiguous_returns_best_not_arbitrary() {
        let near = || {
            vec![
                voice_log(),
                cmd("open chart logs", "OpenChartLogs", CommandClass::Fixed),
            ]
        };
        // (a) exact beats fuzzy, whatever the command order.
        let m = matcher(near(), HashMap::new());
        assert_eq!(
            token(&m, "open chart log").as_deref(),
            Some("OpenChartLogPanel")
        );
        let mut reversed = near();
        reversed.reverse();
        let m = matcher(reversed, HashMap::new());
        assert_eq!(
            token(&m, "open chart log").as_deref(),
            Some("OpenChartLogPanel")
        );

        // (b) equal confidence, different tokens.
        let m = matcher(
            vec![
                cmd("close panel", "ClosePanelA", CommandClass::Fixed),
                cmd("[close; shut] panel", "ClosePanelB", CommandClass::Fixed),
            ],
            HashMap::new(),
        );
        assert_eq!(m.match_text("close panel"), None);

        // (c) both match, both below threshold.
        let m = Matcher::new(near(), HashMap::new(), AliasMap::new(), 0.95);
        assert_eq!(m.match_text("open charr log"), None);
    }

    #[test]
    fn test_below_threshold_returns_none() {
        let m = matcher(vec![voice_log()], HashMap::new());
        assert_eq!(m.match_text("the quick brown fox"), None);
        assert_eq!(m.match_text(""), None);
        assert_eq!(m.match_text("   "), None);
    }

    #[test]
    fn test_alias_match() {
        let mut aliases = AliasMap::new();
        aliases.insert(
            "OpenChartLogPanel".to_string(),
            vec!["kartenbuch [öffnen; zeigen;]".to_string()],
        );
        let m = Matcher::new(vec![voice_log()], HashMap::new(), aliases, 0.8);
        let hit = m.match_text("Kartenbuch ÖFFNEN").unwrap();
        assert_eq!(hit.token, "OpenChartLogPanel");
        assert_eq!(hit.confidence, 1.0);
    }

    #[test]
    fn test_alias_without_phrase_matches() {
        let mut aliases = AliasMap::new();
        aliases.insert(
            SET_DEST.to_string(),
            vec!["kurs setzen nach <System>".to_string()],
        );
        let command = Command {
            phrase: None,
            ..cmd("", SET_DEST, CommandClass::DynamicName)
        };
        let m = Matcher::new(vec![command], vocab(&[("System", &["Ceos"])]), aliases, 0.8);
        assert_eq!(
            token(&m, "Kurs setzen nach Ceos").as_deref(),
            Some("SailTo:\"Ceos\"")
        );
    }

    #[test]
    fn test_alias_cannot_build_cmd_template() {
        let mut aliases = AliasMap::new();
        aliases.insert(
            "Lore:{CMD}".to_string(),
            vec!["erzähl mir von {TXT:LoreType49}".to_string()],
        );
        let m = Matcher::new(
            vec![cmd(
                "[tell us of;] {TXT:LoreType49}",
                "Lore:{CMD}",
                CommandClass::DynamicName,
            )],
            vocab(&[("LoreType49", &["sunken wrecks"])]),
            aliases,
            0.8,
        );
        assert_eq!(m.match_text("erzähl mir von sunken wrecks"), None);
    }

    /// The alias keys that no command takes: each reason of `Matcher::new`
    /// to skip an alias.
    #[test]
    fn test_ignored_alias_keys_name_each_skip_reason() {
        let phrases = |p: &str| vec![p.to_string()];
        let aliases = AliasMap::from([
            // Works.
            ("OpenChartLogPanel".to_string(), phrases("kartenbuch")),
            // No command has this token.
            ("NoSuchToken".to_string(), phrases("gibt es nicht")),
            // A prefix of a composite group.
            ("PrefixToken".to_string(), phrases("vorsilbe")),
            // A value command without a number variable.
            ("SetThrottle".to_string(), phrases("schub auf [0..100]")),
            // The raw template of a command with a constant.
            (SPEC_COURSE.to_string(), phrases("kurs nach graymouth")),
            // A template that needs the spoken English command.
            (
                "Lore:{CMD}".to_string(),
                phrases("erzähl mir von {TXT:LoreType49}"),
            ),
        ]);
        let prefix = Command {
            command_type: CommandType::Prefix,
            ..cmd("dock sell", "PrefixToken", CommandClass::Fixed)
        };
        let commands = vec![
            voice_log(),
            prefix,
            cmd(
                "set throttle to [0..100]",
                "SetThrottle",
                CommandClass::DynamicValue,
            ),
            with_system(
                cmd(
                    "sail out for Graymouth",
                    SPEC_COURSE,
                    CommandClass::DynamicName,
                ),
                "Dunmore",
            ),
            cmd(
                "[tell us of;] {TXT:LoreType49}",
                "Lore:{CMD}",
                CommandClass::DynamicName,
            ),
        ];
        let m = Matcher::new(commands, HashMap::new(), aliases, 0.8);
        assert_eq!(
            m.ignored_alias_keys(),
            [
                SPEC_COURSE,
                "Lore:{CMD}",
                "NoSuchToken",
                "PrefixToken",
                "SetThrottle",
            ]
        );
    }

    #[test]
    fn test_dynamic_value_not_emitted() {
        let mut aliases = AliasMap::new();
        aliases.insert(
            "SetThrottle".to_string(),
            vec!["schub auf [0..100]".to_string()],
        );
        let m = Matcher::new(
            vec![cmd(
                "set throttle to [0..100]",
                "SetThrottle",
                CommandClass::DynamicValue,
            )],
            HashMap::new(),
            aliases,
            0.8,
        );
        assert_eq!(m.match_text("set throttle to 50"), None);
        assert_eq!(m.match_text("schub auf 50"), None);
    }

    #[test]
    fn test_fuzzy_single_word_slop() {
        let m = matcher(vec![voice_log()], HashMap::new());
        let hit = m.match_text("open charr log").unwrap();
        assert_eq!(hit.token, "OpenChartLogPanel");
        assert!(hit.confidence >= 0.8 && hit.confidence < 1.0);
    }

    #[test]
    fn test_leftover_words_return_none() {
        let m = matcher(vec![voice_log()], HashMap::new());
        assert_eq!(m.match_text("open chart log please now"), None);
    }

    /// A text that names a command that needs a value blocks: a lower
    /// candidate of another token does not win. A clearly better candidate
    /// still wins.
    #[test]
    fn test_path_without_a_token_blocks_a_lower_candidate() {
        let m = matcher(
            vec![
                cmd(
                    "lock lantern",
                    "LanternLock={TXT:State}",
                    CommandClass::Fixed,
                ),
                cmd("lock lanterns", "LanternsLock", CommandClass::Fixed),
            ],
            HashMap::new(),
        );
        assert_eq!(
            m.match_result("lock lantern"),
            MatchResult::Blocked(BlockReason::NoToken)
        );
        assert_eq!(token(&m, "lock lanterns").as_deref(), Some("LanternsLock"));
        // Below the threshold: no candidate.
        assert_eq!(m.match_result("list lanes"), MatchResult::NoCandidate);
    }

    #[test]
    fn test_unresolved_placeholder_returns_none() {
        let m = matcher(
            vec![cmd(
                "remove buoy [1..6] [1..6] from {TXT:Group25}",
                "RemoveBuoyFromGroup={TXT:Group25}:{TXT:MooringCoord}",
                CommandClass::DynamicName,
            )],
            vocab(&[("Group25", &["Favourites"])]),
        );
        assert_eq!(m.match_text("remove buoy 1 2 from favourites"), None);
    }

    #[test]
    fn test_cmd_canonical_text() {
        let m = matcher(
            vec![cmd(
                "[tell us of;] {TXT:LoreType49} [1..30;]",
                "Lore:{CMD}",
                CommandClass::DynamicName,
            )],
            vocab(&[("LoreType49", &["sunken wrecks"])]),
        );
        assert_eq!(
            token(&m, "tell us of sunken wrecks 3").as_deref(),
            Some("Lore:tell us of sunken wrecks 3")
        );
    }

    #[test]
    fn test_slot_without_vocabulary_never_matches() {
        let m = matcher(
            vec![
                cmd("sail out to <System>", SET_DEST, CommandClass::DynamicName),
                cmd(
                    "jump to <Empty>",
                    "Jump:{TXT:Empty}",
                    CommandClass::DynamicName,
                ),
            ],
            vocab(&[("Empty", &[])]),
        );
        assert_eq!(m.match_text("sail out to ceos"), None);
        assert_eq!(m.match_text("jump to ceos"), None);
    }

    #[test]
    fn test_number_mismatch_is_not_fuzzy() {
        let m = matcher(
            vec![cmd(
                "sail out to <System>",
                SET_DEST,
                CommandClass::DynamicName,
            )],
            vocab(&[("System", &["TRV 48213"])]),
        );
        assert_eq!(m.match_text("sail out to TRV 48214"), None);
    }

    #[test]
    fn test_tied_slot_entries_return_none() {
        // "ceos" is one edit from both systems: two different tokens tie.
        let m = matcher(
            vec![
                cmd(
                    "sail out to <System1>",
                    "Jump:{TXT:System1}",
                    CommandClass::DynamicName,
                ),
                cmd(
                    "sail out to <System2>",
                    "Jump:{TXT:System2}",
                    CommandClass::DynamicName,
                ),
            ],
            vocab(&[("System1", &["Cios"]), ("System2", &["Ceas"])]),
        );
        assert_eq!(m.match_text("sail out to ceos"), None);
    }

    fn system_specific(slot: &str) -> Command {
        cmd(
            &format!("[sail out for; sail out to] {{TXT:{slot}}}"),
            "HarbourInfo:{TXT:SystemName}:sail out",
            CommandClass::DynamicName,
        )
    }

    #[test]
    fn test_slot_emits_first_entry_whichever_entry_was_spoken() {
        let m = matcher(
            vec![system_specific("System1")],
            vocab(&[(
                "System1",
                &["KDS 4021", "Kads 4021", "Harriet Quell Station"],
            )]),
        );
        assert_eq!(
            token(&m, "sail out to harriet quell station").as_deref(),
            Some("HarbourInfo:KDS 4021:sail out")
        );
        assert_eq!(
            token(&m, "sail out to kads 4021").as_deref(),
            Some("HarbourInfo:KDS 4021:sail out")
        );
    }

    #[test]
    fn test_cmd_uses_spoken_entry_not_first_entry() {
        let m = matcher(
            vec![cmd(
                "[tell us of;] {TXT:LoreType49}",
                "Lore:{CMD}",
                CommandClass::DynamicName,
            )],
            vocab(&[("LoreType49", &["sunken wrecks", "drowned wrecks"])]),
        );
        assert_eq!(
            token(&m, "tell us of drowned wrecks").as_deref(),
            Some("Lore:tell us of drowned wrecks")
        );
    }

    #[test]
    fn test_hyphenated_system_name_matches() {
        let m = matcher(
            vec![system_specific("System2")],
            vocab(&[("System2", &["Brannock KX-E c21-6", "Brannock KX E c21 6"])]),
        );
        assert_eq!(
            token(&m, "sail out to Brannock KX-E c21-6").as_deref(),
            Some("HarbourInfo:Brannock KX-E c21-6:sail out")
        );
    }

    #[test]
    fn test_cmd_template_with_canonical_tie_returns_none() {
        let m = matcher(
            vec![cmd(
                "[tell us of;] {TXT:LoreType49}",
                "Lore:{CMD}",
                CommandClass::DynamicName,
            )],
            vocab(&[(
                "LoreType49",
                &["Brannock KX-E c21-6", "Brannock KX E c21 6"],
            )]),
        );
        assert_eq!(m.match_text("tell us of Brannock KX-E c21-6"), None);
    }

    #[test]
    fn test_literal_route_tie_with_equal_captures_matches() {
        // "let's" and "lets" normalise to the same word; only the canonical text differs.
        let m = matcher(
            vec![cmd(
                "[let's; lets] go to {TXT:System1}",
                "Jump:{TXT:System1}",
                CommandClass::DynamicName,
            )],
            vocab(&[("System1", &["Ceos"])]),
        );
        assert_eq!(token(&m, "let's go to ceos").as_deref(), Some("Jump:Ceos"));
    }

    #[test]
    fn test_set_dynamic_vocab_replaces_names() {
        let mut m = matcher(
            vec![system_specific("System1")],
            vocab(&[("System1", &["Ceos"])]),
        );
        assert!(m.match_text("sail out to ceos").is_some());
        assert_eq!(m.match_text("sail out to dunmore"), None);
        m.set_dynamic_vocab(vocab(&[("System1", &["Dunmore"])]));
        assert_eq!(m.match_text("sail out to ceos"), None);
        assert!(m.match_text("sail out to dunmore").is_some());
    }

    #[test]
    fn test_unclosed_brace_in_template_never_matches() {
        let m = matcher(
            vec![cmd("jump", "X:{TXT:Foo", CommandClass::Fixed)],
            HashMap::new(),
        );
        assert_eq!(m.match_text("jump"), None);
    }

    const TRADE_TEMPLATE: &str = "{TXT:Side}{TXT:Kind}Cork";

    fn prefix(phrase: &str, group: &str, kind: &str) -> Command {
        Command {
            command_type: CommandType::Prefix,
            composite_group: Some(group.to_string()),
            sets: HashMap::from([("Kind".to_string(), kind.to_string())]),
            ..cmd(phrase, "", CommandClass::Fixed)
        }
    }

    fn suffix(phrase: &str, group: &str) -> Command {
        let rule = |contains: &str, side: &str| PrefixRule {
            contains: contains.to_string(),
            set: HashMap::from([("SIDE".to_string(), side.to_string())]),
        };
        Command {
            command_type: CommandType::Suffix,
            composite_group: Some(group.to_string()),
            prefix_rules: vec![rule("buy", "Buy"), rule("sell", "Sell")],
            ..cmd(phrase, TRADE_TEMPLATE, CommandClass::DynamicValue)
        }
    }

    fn trade_matcher() -> Matcher {
        matcher(
            vec![
                prefix("[give me;] dock [buy; sell; trade]", "trade", "Dock:"),
                prefix("[give me;] top [buy; sell]", "trade", "Top:"),
                suffix("[price for;] cork", "trade"),
            ],
            HashMap::new(),
        )
    }

    fn blocking(phrase: &str, command_type: CommandType, group: Option<&str>) -> BlockingPhrase {
        BlockingPhrase {
            phrase: phrase.to_string(),
            class: crate::profile::LeftOutClass::NotUnderstood,
            command_type,
            composite_group: group.map(str::to_string),
        }
    }

    /// The phrase of a left-out command is a path without a token: its
    /// text blocks, also when a fuzzy word reaches a working command. The
    /// working command still matches its own text.
    #[test]
    fn test_a_left_out_phrase_blocks_a_fuzzy_match_of_another_command() {
        let working = vec![cmd("lock lanterns", "LanternsLock", CommandClass::Fixed)];
        let without = matcher(working.clone(), HashMap::new());
        // Without the blocking phrase: the fuzzy word gives the other
        // command. This is the wrong match that the blocking phrase stops.
        assert_eq!(
            token(&without, "lock lantern").as_deref(),
            Some("LanternsLock")
        );
        let m = matcher(working, HashMap::new()).with_blocking(&[blocking(
            "lock lantern",
            CommandType::Full,
            None,
        )]);
        assert_eq!(
            m.match_result("lock lantern"),
            MatchResult::Blocked(BlockReason::NoToken)
        );
        assert_eq!(token(&m, "lock lanterns").as_deref(), Some("LanternsLock"));
        assert_eq!(m.match_result("list lanes"), MatchResult::NoCandidate);
        // The words of the phrase are command words.
        assert!(m.command_words().contains(&"lantern".to_string()));
    }

    /// A left-out suffix joins its group: "dock sell" with the left-out
    /// suffix blocks, the working suffix still matches.
    #[test]
    fn test_a_left_out_suffix_blocks_in_its_composite_group() {
        let parts = vec![
            prefix("dock [buy; sell]", "trade", "Dock:"),
            suffix("cork", "trade"),
        ];
        let without = matcher(parts.clone(), HashMap::new());
        assert_eq!(
            token(&without, "dock sell corks").as_deref(),
            Some("SellDock:Cork")
        );
        let m = matcher(parts, HashMap::new()).with_blocking(&[blocking(
            "corks",
            CommandType::Suffix,
            Some("trade"),
        )]);
        assert_eq!(
            m.match_result("dock sell corks"),
            MatchResult::Blocked(BlockReason::NoToken)
        );
        assert_eq!(
            token(&m, "dock sell cork").as_deref(),
            Some("SellDock:Cork")
        );
        // A suffix alone is no command.
        assert_eq!(m.match_result("corks"), MatchResult::NoCandidate);
    }

    /// A left-out prefix joins its group: each composite with it gives no
    /// token, the working prefix still matches.
    #[test]
    fn test_a_left_out_prefix_blocks_in_its_composite_group() {
        // The token of the suffix needs no variable of the prefix: the
        // rules of the spoken prefix give it. Thus only the flag of the
        // left-out prefix stops the token.
        let side_only = Command {
            token_template: "{TXT:Side}Cork".to_string(),
            ..suffix("cork", "trade")
        };
        let parts = vec![prefix("dock [buy; sell]", "trade", "Dock:"), side_only];
        let without = matcher(parts.clone(), HashMap::new());
        assert_eq!(
            token(&without, "docks sell cork").as_deref(),
            Some("SellCork")
        );
        let m = matcher(parts, HashMap::new()).with_blocking(&[blocking(
            "docks [buy; sell]",
            CommandType::Prefix,
            Some("trade"),
        )]);
        assert_eq!(
            m.match_result("docks sell cork"),
            MatchResult::Blocked(BlockReason::NoToken)
        );
        assert_eq!(token(&m, "dock sell cork").as_deref(), Some("SellCork"));
    }

    /// A group of only left-out parts: a prefix and a suffix pair. A part
    /// of a group without the other side never matches.
    #[test]
    fn test_left_out_parts_of_a_new_group_pair_with_each_other() {
        let working = vec![cmd("harbour sell cork", "HarbourCork", CommandClass::Fixed)];
        let m = matcher(working.clone(), HashMap::new()).with_blocking(&[
            blocking("harbour sell", CommandType::Prefix, Some("quay")),
            blocking("corks", CommandType::Suffix, Some("quay")),
        ]);
        assert_eq!(
            m.match_result("harbour sell corks"),
            MatchResult::Blocked(BlockReason::NoToken)
        );
        let lone = matcher(working, HashMap::new()).with_blocking(&[blocking(
            "corks",
            CommandType::Suffix,
            Some("quay"),
        )]);
        assert_eq!(
            token(&lone, "harbour sell corks").as_deref(),
            Some("HarbourCork")
        );
    }

    #[test]
    fn test_composite_takes_sets_from_prefix_and_rules_from_spoken_prefix() {
        let m = trade_matcher();
        assert_eq!(
            token(&m, "dock sell cork").as_deref(),
            Some("SellDock:Cork")
        );
        assert_eq!(
            token(&m, "give me top buy price for cork").as_deref(),
            Some("BuyTop:Cork")
        );
        // Fuzzy words still count: "corck" is near "cork".
        assert_eq!(
            token(&m, "dock sell corck").as_deref(),
            Some("SellDock:Cork")
        );
    }

    #[test]
    fn test_composite_part_alone_is_no_command() {
        let m = trade_matcher();
        assert_eq!(token(&m, "dock sell"), None);
        assert_eq!(token(&m, "price for cork"), None);
        assert_eq!(token(&m, "cork"), None);
    }

    /// No rule matches "trade": the side stays open (VoiceAttack says
    /// "Unable to comply").
    #[test]
    fn test_composite_with_open_variable_does_not_fire() {
        let m = trade_matcher();
        assert_eq!(token(&m, "dock trade cork"), None);
    }

    #[test]
    fn test_composite_rules_that_disagree_give_no_match() {
        let m = matcher(
            vec![
                prefix("[buy and sell; sell or buy]", "trade", "Dock:"),
                suffix("cork", "trade"),
            ],
            HashMap::new(),
        );
        assert_eq!(token(&m, "buy and sell cork"), None);
    }

    #[test]
    fn test_composite_needs_prefix_and_suffix_of_the_same_group() {
        let m = matcher(
            vec![
                prefix("dock sell", "trade", "Dock:"),
                suffix("cork", "other"),
            ],
            HashMap::new(),
        );
        assert_eq!(token(&m, "dock sell cork"), None);
    }

    /// Two prefixes with the same words but different `sets` give no clear token.
    #[test]
    fn test_composite_same_prefix_words_with_different_sets_give_no_match() {
        let m = matcher(
            vec![
                prefix("dock sell", "trade", "Dock:"),
                prefix("dock sell", "trade", "Top:"),
                suffix("cork", "trade"),
            ],
            HashMap::new(),
        );
        assert_eq!(token(&m, "dock sell cork"), None);
    }

    /// Each part must pass the threshold alone. "cork" -> "cord" has a word
    /// similarity of 0.75: over all words it would pass 0.8, alone it does not.
    #[test]
    fn test_composite_suffix_must_pass_the_threshold_alone() {
        let m = trade_matcher();
        assert_eq!(token(&m, "dock sell cord"), None);
        assert_eq!(token(&m, "dock sell fork"), None);
        let exact = m.match_text("dock sell cork").unwrap();
        assert_eq!(exact.confidence, 1.0);
        assert!(!exact.inexact_single_required_word);
    }

    /// The confidence of a composite is the lower of the prefix part and the
    /// suffix part. A one-word suffix that is not exact is flagged.
    #[test]
    fn test_composite_confidence_is_the_lower_part() {
        let m = trade_matcher();
        // "corks" -> "cork": 0.8.
        let hit = m.match_text("give me dock sell corks").unwrap();
        assert_eq!(hit.token, "SellDock:Cork");
        assert!((hit.confidence - 0.8).abs() < 1e-6, "{}", hit.confidence);
        assert!(hit.inexact_single_required_word);
        // Optional filler words ("price for") do not count: the name is still
        // one word that is not exact, with the same score.
        let hit = m.match_text("dock sell price for corks").unwrap();
        assert!((hit.confidence - 0.8).abs() < 1e-6, "{}", hit.confidence);
        assert!(hit.inexact_single_required_word);
        // Filler with a name below the threshold gives no match.
        assert_eq!(token(&m, "dock sell price for cord"), None);
        // Filler with an exact name is exact.
        let hit = m.match_text("dock sell price for cork").unwrap();
        assert_eq!(hit.confidence, 1.0);
        assert!(!hit.inexact_single_required_word);
    }

    /// A suffix that matched only optional filler words has no name: no match.
    #[test]
    fn test_composite_suffix_with_only_filler_does_not_fire() {
        let m = matcher(
            vec![
                prefix("dock sell", "trade", "Dock:"),
                Command {
                    token_template: "{TXT:Kind}Cork".to_string(),
                    prefix_rules: Vec::new(),
                    ..suffix("[price for;] [cork;]", "trade")
                },
            ],
            HashMap::new(),
        );
        // All suffix words are optional filler, so the name is empty.
        assert_eq!(token(&m, "dock sell price for"), None);
        assert_eq!(token(&m, "dock sell price for cork"), None);
    }

    /// A prefix that can match no words never lets a suffix fire alone.
    #[test]
    fn test_composite_prefix_without_words_does_not_fire() {
        let m = matcher(
            vec![
                prefix("[give me;] [the;]", "trade", "Dock:"),
                Command {
                    token_template: "{TXT:Kind}Cork".to_string(),
                    prefix_rules: Vec::new(),
                    ..suffix("[price for;] cork", "trade")
                },
            ],
            HashMap::new(),
        );
        assert_eq!(token(&m, "cork"), None);
        assert_eq!(token(&m, "price for cork"), None);
    }

    /// A suffix that can match no words never lets a prefix fire alone.
    #[test]
    fn test_composite_suffix_without_words_does_not_fire() {
        let m = matcher(
            vec![
                prefix("dock sell", "trade", "Dock:"),
                Command {
                    token_template: "{TXT:Kind}Cork".to_string(),
                    prefix_rules: Vec::new(),
                    ..suffix("[cork;]", "trade")
                },
            ],
            HashMap::new(),
        );
        assert_eq!(token(&m, "dock sell"), None);
    }

    fn look_for_cork() -> Command {
        cmd(
            "[look for; hunt for; find;] Cork",
            "SetCargo:Cork",
            CommandClass::Fixed,
        )
    }

    /// Optional words do not dilute one misheard required word: "cord" is
    /// 0.75 from "cork", below the threshold.
    #[test]
    fn test_required_words_must_pass_the_threshold_alone() {
        let m = matcher(vec![look_for_cork()], HashMap::new());
        assert_eq!(m.match_text("look for cord"), None);
        let hit = m.match_text("look for cork").unwrap();
        assert_eq!(hit.token, "SetCargo:Cork");
        assert_eq!(hit.confidence, 1.0);
        assert!(!hit.inexact_single_required_word);
    }

    /// One required word that is close but not exact is flagged.
    #[test]
    fn test_one_inexact_required_word_is_flagged() {
        let m = matcher(vec![look_for_cork()], HashMap::new());
        let hit = m.match_text("hunt for corks").unwrap();
        assert_eq!(hit.token, "SetCargo:Cork");
        assert!(hit.inexact_single_required_word);
    }

    /// Two required words with one small error still match, without the flag.
    #[test]
    fn test_two_required_words_with_one_small_error_match() {
        let m = matcher(vec![voice_log()], HashMap::new());
        let hit = m.match_text("open the charr log").unwrap();
        assert_eq!(hit.token, "OpenChartLogPanel");
        assert!(!hit.inexact_single_required_word);
    }

    /// A failed required score blocks the match. A lower candidate does not win.
    #[test]
    fn test_failed_required_score_does_not_let_a_lower_command_win() {
        let m = matcher(
            vec![
                look_for_cork(),
                // 0.85 for "look for cord", all words required.
                cmd("looks fore cord", "Other", CommandClass::Fixed),
            ],
            HashMap::new(),
        );
        assert_eq!(m.match_text("look for cord"), None);
        assert_eq!(
            m.match_result("look for cord"),
            MatchResult::Blocked(BlockReason::RequiredWords)
        );
        assert_eq!(m.match_result("look for zebra"), MatchResult::NoCandidate);
    }

    /// Within one token, a path that passes its required words wins over a
    /// better path that fails them. Another token never wins that way.
    #[test]
    fn test_passing_path_of_the_same_token_wins_but_never_another_token() {
        // "look for cord": 0.92 failing for the first phrase, 0.85 passing
        // for the second phrase, both token A.
        let a = || {
            vec![
                look_for_cork(),
                cmd("looks fore cord", "SetCargo:Cork", CommandClass::Fixed),
            ]
        };
        let m = matcher(a(), HashMap::new());
        let hit = m.match_text("look for cord").unwrap();
        assert_eq!(hit.token, "SetCargo:Cork");
        assert!((hit.confidence - 0.85).abs() < 1e-6, "{}", hit.confidence);
        // Token B (0.87) is better than the passing path of A, but below
        // the failing path of A: no match.
        let mut commands = a();
        commands.push(cmd("lookk for cords", "B", CommandClass::Fixed));
        let m = matcher(commands, HashMap::new());
        assert_eq!(
            m.match_result("look for cord"),
            MatchResult::Blocked(BlockReason::RequiredWords)
        );
    }

    /// A phrase of only optional words has no required words: the overall
    /// confidence decides.
    #[test]
    fn test_phrase_with_only_optional_words() {
        let m = matcher(
            vec![cmd("[open;] [the;] [panel;]", "P", CommandClass::Fixed)],
            HashMap::new(),
        );
        let hit = m.match_text("open the panel").unwrap();
        assert_eq!(hit.confidence, 1.0);
        assert_eq!(hit.required_words, 0);
        assert!(!hit.inexact_single_required_word);
        let hit = m.match_text("opan the panel").unwrap();
        assert!(!hit.inexact_single_required_word);
    }

    /// Words in nested optional groups are filler too.
    #[test]
    fn test_nested_optional_words_are_not_required() {
        let m = matcher(
            vec![cmd("[open [the;];] signal chart", "G", CommandClass::Fixed)],
            HashMap::new(),
        );
        let hit = m.match_text("opan the signal chart").unwrap();
        assert_eq!(hit.required_words, 2);
        assert_eq!(hit.required_positions, vec![2, 3]);
        assert!(!hit.inexact_single_required_word);
        let m = matcher(
            vec![cmd("[open [the;];] signals", "G", CommandClass::Fixed)],
            HashMap::new(),
        );
        let hit = m.match_text("open the signels").unwrap();
        assert_eq!(hit.required_words, 1);
        assert!(hit.inexact_single_required_word);
    }

    /// A number in an optional group is filler.
    #[test]
    fn test_range_inside_an_optional_group_is_not_required() {
        let m = number_matcher(number_cmd("set [1..9;] marker", "Marker:{TXT:N}", &["N"]));
        let hit = m.match_text("set 3 marker").unwrap();
        assert_eq!(hit.token, "Marker:3");
        assert_eq!(hit.required_words, 2);
        assert_eq!(hit.required_positions, vec![0, 2]);
    }

    /// A one-word system name that is not exact is flagged.
    #[test]
    fn test_one_word_system_name_matched_fuzzily_is_flagged() {
        let m = matcher(
            vec![cmd(
                "[sail out to;] {TXT:System1}",
                "Jump:{TXT:System1}",
                CommandClass::DynamicName,
            )],
            vocab(&[("System1", &["Ceos"])]),
        );
        let hit = m.match_text("sail out to ceoss").unwrap();
        assert_eq!(hit.token, "Jump:Ceos");
        assert!(hit.inexact_single_required_word);
        let hit = m.match_text("sail out to ceos").unwrap();
        assert!(!hit.inexact_single_required_word);
    }

    /// A slot name is scored on its own: two words at 0.75 fail, although
    /// the whole phrase reaches 0.9.
    #[test]
    fn test_slot_name_must_pass_the_threshold_alone() {
        let m = matcher(
            vec![cmd(
                "[sail out to; sail out for] {TXT:System1}",
                "Jump:{TXT:System1}",
                CommandClass::DynamicName,
            )],
            vocab(&[("System1", &["Wolf Bell"])]),
        );
        assert_eq!(
            m.match_result("sail out to wolt belt"),
            MatchResult::Blocked(BlockReason::RequiredWords)
        );
        let hit = m.match_text("sail out to wolf bell").unwrap();
        assert_eq!(hit.single_word_slot_positions, Vec::<usize>::new());
        // Each word must pass alone too: "belt" is 0.75 from "bell".
        assert_eq!(
            m.match_result("sail out to wolf belt"),
            MatchResult::Blocked(BlockReason::RequiredWords)
        );
    }

    /// A multi-word slot name with a small error in a long word still
    /// matches. A weak short word fails, even with a strong long word.
    #[test]
    fn test_each_word_of_a_slot_name_must_pass_alone() {
        let m = matcher(
            vec![cmd(
                "[sail out to; sail out for] {TXT:System1}",
                "Jump:{TXT:System1}",
                CommandClass::DynamicName,
            )],
            vocab(&[("System1", &["Tou Ming", "Pemberton Rock"])]),
        );
        let hit = m.match_text("sail out to pembertin rock").unwrap();
        assert!(!hit.inexact_single_required_word);
        for text in ["sail out to tou ling", "sail out to pemberton rack"] {
            assert_eq!(
                m.match_result(text),
                MatchResult::Blocked(BlockReason::RequiredWords),
                "{text}"
            );
        }
    }

    /// Fuzzy matches without optional words keep the same confidence.
    #[test]
    fn test_confidence_without_optional_words_does_not_change() {
        let cases = [
            (
                "light the lanterns",
                "light the lantern",
                &["lanterns", "lantern"],
            ),
            (
                "request mooring",
                "request mooreng",
                &["mooring", "mooreng"],
            ),
            (
                "signal next harbour",
                "signal nex harbour",
                &["next", "nex"],
            ),
        ];
        for (phrase, spoken, [a, b]) in cases {
            let m = matcher(vec![cmd(phrase, "T", CommandClass::Fixed)], HashMap::new());
            let words = phrase.split_whitespace().count() as f64;
            let expected = ((words - 1.0 + strsim::normalized_levenshtein(a, b)) / words) as f32;
            let hit = m.match_text(spoken).unwrap();
            assert_eq!(hit.confidence, expected, "{spoken}");
            assert!(!hit.inexact_single_required_word, "{spoken}");
        }
    }

    /// A full command with the same words and a different token is a tie.
    #[test]
    fn test_composite_and_full_command_with_same_words_tie() {
        let mut commands = vec![
            prefix("dock sell", "trade", "Dock:"),
            suffix("cork", "trade"),
        ];
        commands.push(cmd("dock sell cork", "Other", CommandClass::Fixed));
        let m = matcher(commands, HashMap::new());
        assert_eq!(token(&m, "dock sell cork"), None);
    }

    /// The margin check uses the path that is sent. The best path of token
    /// A (0.9375) fails its required words. The passing path of A (0.846)
    /// is below the path without a token (0.914): no match.
    #[test]
    fn test_path_without_a_token_blocks_the_passing_path_that_is_sent() {
        let rope = || {
            cmd(
                "hoist the harbor cords",
                "Rope={TXT:Length}",
                CommandClass::DynamicValue,
            )
        };
        let a = || {
            vec![
                // "cord" is 0.75 from "cork": the required word fails.
                cmd(
                    "[hoist the harbour;] cork",
                    "Cargo:Cork",
                    CommandClass::Fixed,
                ),
                cmd(
                    "hoists thee harbour cords",
                    "Cargo:Cork",
                    CommandClass::Fixed,
                ),
            ]
        };
        let mut commands = a();
        commands.push(rope());
        // A token of another command, lower than every path above.
        commands.push(cmd(
            "hoists thee harbour corks",
            "Other",
            CommandClass::Fixed,
        ));
        let m = matcher(commands, HashMap::new());
        assert_eq!(
            m.match_result("hoist the harbour cord"),
            MatchResult::Blocked(BlockReason::NoToken)
        );
        // Without the command that needs a value, the passing path of A wins.
        let m = matcher(a(), HashMap::new());
        let hit = m.match_text("hoist the harbour cord").unwrap();
        assert_eq!(hit.token, "Cargo:Cork");
        assert!((hit.confidence - 0.8458).abs() < 1e-3, "{}", hit.confidence);
    }

    /// The text of the tests of the paths without a token. "mainsail" is
    /// 0.889 from "mainsails": each phrase "trim the mainsails" reaches
    /// 0.963 on it.
    const TRIM: &str = "trim the mainsail";

    /// 0.85 on `TRIM`: lower than the path without a token.
    fn lower_trim() -> Command {
        cmd("trims thee mainsail", "Lower", CommandClass::Fixed)
    }

    /// 1.0 on `TRIM`: better than the path without a token.
    fn better_trim() -> Command {
        cmd("trim the mainsail", "Better", CommandClass::Fixed)
    }

    /// A text that names a path without a token gives no match, also with a
    /// lower candidate of another token. A candidate that is better by more
    /// than the tie margin wins. Without the path, the lower candidate wins.
    fn check_path_without_a_token(
        source: &str,
        token_free: Vec<Command>,
        vocab: HashMap<String, Vec<String>>,
        aliases: AliasMap,
        text: &str,
        lower: Command,
        better: Command,
    ) {
        let build = |extra: Option<Command>| {
            let mut commands = token_free.clone();
            commands.extend(extra);
            Matcher::new(commands, vocab.clone(), aliases.clone(), 0.8)
        };
        let blocked = MatchResult::Blocked(BlockReason::NoToken);
        assert_eq!(build(None).match_result(text), blocked, "{source}: alone");
        assert_eq!(
            build(Some(lower.clone())).match_result(text),
            blocked,
            "{source}: with a lower candidate"
        );
        let better_token = better.token_template.clone();
        assert_eq!(
            token(&build(Some(better)), text),
            Some(better_token),
            "{source}: with a better candidate"
        );
        let alone = Matcher::new(vec![lower.clone()], vocab, aliases, 0.8);
        assert_eq!(
            token(&alone, text),
            Some(lower.token_template),
            "{source}: the lower candidate alone"
        );
    }

    /// The branch of `Matcher::new` for a value command without a number
    /// variable: its phrase stays as a grammar that never gives a token.
    #[test]
    fn test_value_command_without_a_number_variable_keeps_a_grammar_without_a_token() {
        let value = cmd(
            "trim the mainsails",
            "Sail={TXT:Trim}",
            CommandClass::DynamicValue,
        );
        let m = matcher(vec![value.clone()], HashMap::new());
        assert_eq!(m.grammars.len(), 1);
        assert!(m.grammars[0].no_token);
        // The same command with a number variable gets a grammar that can
        // give a token.
        let number = number_cmd("trim the mainsails [1..9]", "Sail={TXT:N}", &["N"]);
        let m = matcher(vec![number], HashMap::new());
        assert!(!m.grammars[0].no_token);
        assert_eq!(token(&m, "trim the mainsails 3").as_deref(), Some("Sail=3"));
        check_path_without_a_token(
            "value command without a number variable",
            vec![value],
            HashMap::new(),
            AliasMap::new(),
            TRIM,
            lower_trim(),
            better_trim(),
        );
    }

    /// A number command whose optional number was not spoken: no number,
    /// no token.
    #[test]
    fn test_number_command_without_its_optional_number_blocks() {
        let number = number_cmd("trim the mainsails [1..9;]", "Sail={TXT:N}", &["N"]);
        let m = matcher(vec![number.clone()], HashMap::new());
        assert_eq!(token(&m, "trim the mainsails 4").as_deref(), Some("Sail=4"));
        check_path_without_a_token(
            "number command without its optional number",
            vec![number],
            HashMap::new(),
            AliasMap::new(),
            TRIM,
            lower_trim(),
            better_trim(),
        );
    }

    /// An alias path of a `{CMD}` template: the alias does not give the
    /// English command text.
    #[test]
    fn test_alias_path_of_a_spoken_command_template_blocks() {
        let flag = cmd(
            "[hoist; lower] the flag",
            "Flag:{CMD}",
            CommandClass::DynamicName,
        );
        let aliases = AliasMap::from([(
            "Flag:{CMD}".to_string(),
            vec!["trim the mainsails".to_string()],
        )]);
        let m = Matcher::new(vec![flag.clone()], HashMap::new(), aliases.clone(), 0.8);
        assert_eq!(
            token(&m, "hoist the flag").as_deref(),
            Some("Flag:hoist the flag")
        );
        check_path_without_a_token(
            "alias path of a {CMD} template",
            vec![flag],
            HashMap::new(),
            aliases,
            TRIM,
            lower_trim(),
            better_trim(),
        );
    }

    /// Two slots with the same name text give the same score with other
    /// captures: the capture is not clear.
    #[test]
    fn test_captures_that_are_not_clear_block() {
        let sail = cmd(
            "trim the [{TXT:Sail1}; {TXT:Sail2}]",
            "Sail:{TXT:Sail1}",
            CommandClass::DynamicName,
        );
        // One slot has the name: the capture is clear.
        let clear = vocab(&[("Sail1", &["mainsails"]), ("Sail2", &["foresails"])]);
        let m = matcher(vec![sail.clone()], clear);
        assert_eq!(token(&m, TRIM).as_deref(), Some("Sail:mainsails"));
        check_path_without_a_token(
            "captures that are not clear",
            vec![sail],
            vocab(&[("Sail1", &["mainsails"]), ("Sail2", &["mainsails"])]),
            AliasMap::new(),
            TRIM,
            lower_trim(),
            better_trim(),
        );
    }

    /// Two alternatives with the same words and other text give the same
    /// score with other command text: `{CMD}` is not clear.
    #[test]
    fn test_command_text_that_is_not_clear_blocks() {
        let sail = cmd(
            "trim the [Mainsails; mainsails]",
            "Sail:{CMD}",
            CommandClass::DynamicName,
        );
        let m = matcher(
            vec![cmd(
                "trim the mainsails",
                "Sail:{CMD}",
                CommandClass::DynamicName,
            )],
            HashMap::new(),
        );
        assert_eq!(token(&m, TRIM).as_deref(), Some("Sail:trim the mainsails"));
        check_path_without_a_token(
            "command text that is not clear",
            vec![sail],
            HashMap::new(),
            AliasMap::new(),
            TRIM,
            lower_trim(),
            better_trim(),
        );
    }

    /// A composite suffix whose prefix rules disagree: "buy and sell" sets
    /// the side to Buy and to Sell. The path reaches 0.917 ("sel" is 0.75
    /// from "sell").
    #[test]
    fn test_composite_suffix_with_rules_that_disagree_blocks() {
        check_path_without_a_token(
            "composite suffix with rules that disagree",
            vec![
                prefix("buy and sell", "trade", "Dock:"),
                suffix("cork", "trade"),
            ],
            HashMap::new(),
            AliasMap::new(),
            "buy and sel cork",
            // 0.8875.
            cmd("buys and sel corks", "Lower", CommandClass::Fixed),
            cmd("buy and sel cork", "Better", CommandClass::Fixed),
        );
    }

    /// A template with a placeholder that no path fills.
    #[test]
    fn test_unresolved_placeholder_blocks() {
        check_path_without_a_token(
            "unresolved placeholder",
            vec![cmd(
                "trim the mainsails",
                "Sail={TXT:Trim}",
                CommandClass::Fixed,
            )],
            HashMap::new(),
            AliasMap::new(),
            TRIM,
            lower_trim(),
            better_trim(),
        );
    }

    /// A working phrase that is the same as the phrase of a command that
    /// needs a value gives no match.
    #[test]
    fn test_same_working_phrase_and_phrase_without_a_token_block() {
        let m = matcher(
            vec![
                cmd(
                    "trim the mainsails",
                    "Sail={TXT:Trim}",
                    CommandClass::DynamicValue,
                ),
                cmd("trim the mainsails", "SailTrim", CommandClass::Fixed),
            ],
            HashMap::new(),
        );
        assert_eq!(
            m.match_result("trim the mainsails"),
            MatchResult::Blocked(BlockReason::NoToken)
        );
    }

    /// An alias path of a working command and the phrase of a command that
    /// needs a value: no match.
    #[test]
    fn test_working_alias_path_and_phrase_without_a_token_block() {
        let aliases = || {
            AliasMap::from([(
                "Boathouse".to_string(),
                vec!["trim the mainsails".to_string()],
            )])
        };
        let boathouse = || cmd("open the boathouse", "Boathouse", CommandClass::Fixed);
        let m = Matcher::new(vec![boathouse()], HashMap::new(), aliases(), 0.8);
        assert_eq!(
            token(&m, "trim the mainsails").as_deref(),
            Some("Boathouse")
        );
        let value = cmd(
            "trim the mainsails",
            "Sail={TXT:Trim}",
            CommandClass::DynamicValue,
        );
        let m = Matcher::new(vec![boathouse(), value], HashMap::new(), aliases(), 0.8);
        assert_eq!(
            m.match_result("trim the mainsails"),
            MatchResult::Blocked(BlockReason::NoToken)
        );
    }

    /// A working composite command and the phrase of a command that needs a
    /// value: no match.
    #[test]
    fn test_working_composite_and_phrase_without_a_token_block() {
        let trade = || {
            vec![
                prefix("dock sell", "trade", "Dock:"),
                suffix("cork", "trade"),
            ]
        };
        assert_eq!(
            token(&matcher(trade(), HashMap::new()), "dock sell cork").as_deref(),
            Some("SellDock:Cork")
        );
        let mut commands = trade();
        commands.push(cmd(
            "dock sell cork",
            "Cork={TXT:Price}",
            CommandClass::DynamicValue,
        ));
        assert_eq!(
            matcher(commands, HashMap::new()).match_result("dock sell cork"),
            MatchResult::Blocked(BlockReason::NoToken)
        );
    }

    /// A candidate of the decision with one confidence.
    fn candidate(confidence: f64, required_ok: bool) -> Candidate {
        Candidate {
            confidence,
            inexact_single_required_word: false,
            required_ok,
            required_positions: Vec::new(),
            required_similarities: Vec::new(),
            single_word_slot_positions: Vec::new(),
            slot_positions: Vec::new(),
        }
    }

    /// The best path and the best passing path of one token.
    fn token_candidates(best: Candidate, passing: Option<Candidate>) -> TokenCandidates {
        TokenCandidates {
            best,
            best_passing: passing,
        }
    }

    /// The tie margin above the path without a token: inside the margin
    /// blocks, outside it the token wins. The margin applies to the path
    /// that is sent.
    #[test]
    fn test_tie_margin_above_a_path_without_a_token() {
        let free = WithoutToken {
            passing: 0.9,
            any: 0.9,
        };
        let one = |c: Candidate, passing: Option<Candidate>| {
            BTreeMap::from([("A".to_string(), token_candidates(c, passing))])
        };
        let blocked = MatchResult::Blocked(BlockReason::NoToken);
        for inside in [free.passing, free.passing + TIE_EPSILON / 2.0] {
            let c = candidate(inside, true);
            assert_eq!(decide(&one(c.clone(), Some(c)), free, 0.8), blocked);
        }
        let outside = free.passing + TIE_EPSILON * 2.0;
        let c = candidate(outside, true);
        let MatchResult::Match(hit) = decide(&one(c.clone(), Some(c)), free, 0.8) else {
            panic!("expected a match");
        };
        assert_eq!(hit.token, "A");
        // The best path fails its required words. The passing path is sent:
        // the margin uses it.
        let best = candidate(0.95, false);
        let inside = candidate(free.passing + TIE_EPSILON / 2.0, true);
        assert_eq!(decide(&one(best.clone(), Some(inside)), free, 0.8), blocked);
        let passing = candidate(outside, true);
        let MatchResult::Match(hit) = decide(&one(best, Some(passing)), free, 0.8) else {
            panic!("expected a match");
        };
        assert_eq!(hit.confidence, outside as f32);
        // No path without a token: the token wins.
        let c = candidate(free.passing, true);
        assert!(matches!(
            decide(&one(c.clone(), Some(c)), WithoutToken::none(), 0.8),
            MatchResult::Match(_)
        ));
        // A path without a token that fails its required words: inside the
        // margin of the sent path it blocks, below it the token wins.
        let failing = |any: f64| WithoutToken {
            passing: f64::NEG_INFINITY,
            any,
        };
        let c = candidate(0.9, true);
        assert_eq!(
            decide(&one(c.clone(), Some(c.clone())), failing(0.9), 0.8),
            blocked
        );
        assert_eq!(
            decide(&one(c.clone(), Some(c.clone())), failing(0.95), 0.8),
            blocked
        );
        assert!(matches!(
            decide(&one(c.clone(), Some(c)), failing(0.85), 0.8),
            MatchResult::Match(_)
        ));
    }

    /// A group with a working prefix and only left-out suffixes: the
    /// left-out suffixes pair with the working prefix, so the text of the
    /// composite blocks. Without the pair, a full command one fuzzy word
    /// away takes the text.
    #[test]
    fn test_a_working_prefix_pairs_with_left_out_suffixes() {
        let working = vec![
            prefix("best buy price for", "market", "Top:"),
            cmd("vest buy price for cork", "Other:Vest", CommandClass::Fixed),
        ];
        let m = matcher(working, HashMap::new()).with_blocking(&[
            blocking("cork", CommandType::Suffix, Some("market")),
            blocking("rope", CommandType::Suffix, Some("market")),
        ]);
        for text in ["best buy price for cork", "best buy price for rope"] {
            assert_eq!(
                m.match_result(text),
                MatchResult::Blocked(BlockReason::NoToken),
                "{text}"
            );
        }
        assert_eq!(
            token(&m, "vest buy price for cork").as_deref(),
            Some("Other:Vest")
        );
    }

    /// The reverse: a left-out prefix pairs with the working suffix of its
    /// group.
    #[test]
    fn test_a_left_out_prefix_pairs_with_a_working_suffix() {
        let working = vec![
            suffix("cork", "market"),
            cmd("vest buy price for cork", "Other:Vest", CommandClass::Fixed),
        ];
        let m = matcher(working, HashMap::new()).with_blocking(&[blocking(
            "best buy price for",
            CommandType::Prefix,
            Some("market"),
        )]);
        assert_eq!(
            m.match_result("best buy price for cork"),
            MatchResult::Blocked(BlockReason::NoToken)
        );
        assert_eq!(
            token(&m, "vest buy price for cork").as_deref(),
            Some("Other:Vest")
        );
        // A group with no prefix at all still gives nothing.
        assert_eq!(m.match_result("cork"), MatchResult::NoCandidate);
    }

    /// A path without a token blocks also when its required words fail:
    /// it blocks when its confidence is not below the path that the
    /// matcher would send.
    #[test]
    fn test_a_path_without_a_token_that_fails_its_required_words_blocks() {
        let working = vec![cmd("now sing the well", "Well:Sing", CommandClass::Fixed)];
        let without = matcher(working.clone(), HashMap::new());
        assert_eq!(
            token(&without, "now ring the bell").as_deref(),
            Some("Well:Sing")
        );
        let m = matcher(working, HashMap::new()).with_blocking(&[blocking(
            "[now ring the;] belt",
            CommandType::Full,
            None,
        )]);
        assert_eq!(
            m.match_result("now ring the bell"),
            MatchResult::Blocked(BlockReason::NoToken)
        );
        assert_eq!(token(&m, "now sing the well").as_deref(), Some("Well:Sing"));
    }
}
