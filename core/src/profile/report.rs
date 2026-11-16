//! The report of a build: facts about the source, the format note, the
//! counts, and the commands that the builder left out. Also the difference
//! of two reports, for a person who wants to know what a new profile
//! changed.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;

use super::record::{ObservedCounts, TESTED_FIELD_COUNTS};

/// The most counts of [`BuildReport::unknown_settings`]: the largest
/// counts stay.
pub(crate) const MAX_REPORT_SETTINGS: usize = 256;

/// The counts of `tally` for the report: at most [`MAX_REPORT_SETTINGS`]
/// entries, the largest counts first, and the field order for equal
/// counts.
pub(crate) fn report_settings(
    tally: &BTreeMap<(i32, usize), usize>,
) -> BTreeMap<(i32, usize), usize> {
    let mut all: Vec<(&(i32, usize), &usize)> = tally.iter().collect();
    all.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
    all.into_iter()
        .take(MAX_REPORT_SETTINGS)
        .map(|(k, v)| (*k, *v))
        .collect()
}

/// Why a profile command is not in the map. Each class gives a miss, never
/// a wrong match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LeftOutClass {
    /// The command writes no request token.
    NoWrite,
    /// The command writes two or more request tokens.
    MultiWrite,
    /// A full command whose single write is inside a condition block, or
    /// that also presses a key.
    Maintenance,
    /// A suffix whose composite group has no prefix.
    Orphan,
    /// The command has an action, a shape, or a value that the builder does
    /// not fully understand. The reason names it.
    NotUnderstood,
}

impl LeftOutClass {
    /// The class name. The first four are the names of the first map
    /// extractor (a Python script).
    pub fn as_str(self) -> &'static str {
        match self {
            LeftOutClass::NoWrite => "no_write",
            LeftOutClass::MultiWrite => "multi_write",
            LeftOutClass::Maintenance => "maintenance",
            LeftOutClass::Orphan => "orphan_suffix",
            LeftOutClass::NotUnderstood => "not_understood",
        }
    }
}

/// A profile command that is not in the map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeftOut {
    /// The phrase of the command. Empty when the command has no phrase.
    pub phrase: String,
    pub class: LeftOutClass,
    /// The request tokens of the command, in action order.
    pub tokens: Vec<String>,
    /// A short reason for a person: fixed words and numbers.
    pub reason: String,
}

/// The phrase of a left-out command that VoiceAttack recognises by voice:
/// the command is enabled, has a voice trigger and a spoken phrase, and is
/// not an internal command (a phrase in `((` and `))`). The program sends
/// nothing for it. The matcher keeps it as a path without a token, so that
/// a text that names it does not become another command (a sound-alike, a
/// synonym, a filler drop or a fuzzy word).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockingPhrase {
    /// The phrase. After the load, with the runtime grammar and the grammar
    /// of the profile variables in it.
    pub phrase: String,
    /// The class of the left-out command.
    pub class: LeftOutClass,
    /// A full command, or a prefix or suffix of the composite group
    /// `composite_group`.
    pub command_type: crate::command_map::CommandType,
    pub composite_group: Option<String>,
}

/// Facts about the source profile.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SourceInfo {
    /// The SHA-256 of the `.vap` bytes, in lowercase hex.
    pub sha256: String,
    pub compressed_len: usize,
    pub inflated_len: usize,
    /// The VoiceAttack version text of the profile (profile field 32):
    /// digits and dots.
    pub voiceattack_version: String,
    pub actions: usize,
    pub request_writes: usize,
}

/// How the field counts of the profile compare with the tested format.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum FormatNote {
    /// Each record has the field count of the tested format.
    #[default]
    Tested,
    /// Some records have more fields than the tested format, and none has
    /// fewer.
    NewerThanTested(ObservedCounts),
    /// Some records have fewer fields than the tested format (but enough
    /// for the rules), and none has more.
    OlderThanTested(ObservedCounts),
    /// Some records have more fields and some have fewer.
    Mixed(ObservedCounts),
}

impl FormatNote {
    /// The note for the observed field counts.
    pub fn from_counts(counts: &ObservedCounts) -> FormatNote {
        let t = TESTED_FIELD_COUNTS;
        let pairs = std::iter::once((counts.profile, t.profile))
            .chain(counts.command.iter().map(|&n| (n, t.command)))
            .chain(counts.action.iter().map(|&n| (n, t.action)));
        let (mut more, mut fewer) = (false, false);
        for (found, tested) in pairs {
            more |= found > tested as i64;
            fewer |= found < tested as i64;
        }
        match (more, fewer) {
            (false, false) => FormatNote::Tested,
            (true, false) => FormatNote::NewerThanTested(counts.clone()),
            (false, true) => FormatNote::OlderThanTested(counts.clone()),
            (true, true) => FormatNote::Mixed(counts.clone()),
        }
    }
}

/// The smallest and the largest header count as text: "82" or "82 to 84".
fn list(counts: &BTreeSet<i64>) -> String {
    match (counts.first(), counts.last()) {
        (Some(lo), Some(hi)) if lo != hi => format!("{lo} to {hi}"),
        (Some(lo), _) => lo.to_string(),
        _ => "none".to_string(),
    }
}

impl fmt::Display for FormatNote {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (word, counts) = match self {
            FormatNote::Tested => return f.write_str("the profile has the tested format"),
            FormatNote::NewerThanTested(c) => ("newer than", c),
            FormatNote::OlderThanTested(c) => ("older than", c),
            FormatNote::Mixed(c) => ("different from", c),
        };
        let t = TESTED_FIELD_COUNTS;
        write!(
            f,
            "the profile format is {word} the tested format: field counts profile {} \
             (tested {}), command {} (tested {}), action {} (tested {})",
            counts.profile,
            t.profile,
            list(&counts.command),
            t.command,
            list(&counts.action),
            t.action
        )
    }
}

/// The report of one build. The default is an empty report of a profile
/// without commands. Tests use it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BuildReport {
    pub source: SourceInfo,
    pub format: FormatNote,
    /// The commands in the profile.
    pub commands_in_profile: usize,
    /// The records in the map.
    pub records: usize,
    /// The profile commands that are not in the map, in profile order.
    pub left_out: Vec<LeftOut>,
    /// The phrases of the records in the map.
    pub mapped_phrases: BTreeSet<String>,
    /// The number of actions with a setting that the tested profile does
    /// not have, by (action type, field). A count here shows a drift of the
    /// format before it stops the build. At most 256 counts (the largest).
    pub unknown_settings: BTreeMap<(i32, usize), usize>,
    /// The records with a phrase variable (`{TXT:name}`) that is no slot
    /// of the vocabulary files, no profile variable and no variable of the
    /// runtime grammar (`EDCP_`). Such a record never matches.
    pub unknown_phrase_vars: usize,
    /// The records whose token has a placeholder that no path of the
    /// matcher fills: another VoiceAttack token (for example `{INT:x}`), or
    /// a text variable that no phrase slot, constant, number variable or
    /// prefix rule gives. Such a record never gives a token. `records`
    /// counts them too.
    pub cannot_fire: usize,
}

impl BuildReport {
    /// The number of left-out commands of each class.
    pub fn left_out_count(&self, class: LeftOutClass) -> usize {
        self.left_out.iter().filter(|l| l.class == class).count()
    }

    /// The left-out commands grouped by reason: the reason, the number of
    /// commands, and the phrase of the first command. The largest group
    /// comes first. Groups of the same size are in the order of the reason.
    pub fn reasons(&self) -> Vec<(String, usize, String)> {
        self.group_reasons(|_| true)
    }

    /// `reasons` for the left-out commands of one class.
    pub fn class_reasons(&self, class: LeftOutClass) -> Vec<(String, usize, String)> {
        self.group_reasons(|l| l.class == class)
    }

    fn group_reasons(&self, keep: impl Fn(&LeftOut) -> bool) -> Vec<(String, usize, String)> {
        let mut by_reason: HashMap<&str, (usize, &str)> = HashMap::new();
        for l in self.left_out.iter().filter(|l| keep(l)) {
            by_reason
                .entry(l.reason.as_str())
                .or_insert((0, l.phrase.as_str()))
                .0 += 1;
        }
        let mut groups: Vec<(String, usize, String)> = by_reason
            .into_iter()
            .map(|(reason, (n, phrase))| (reason.to_string(), n, phrase.to_string()))
            .collect();
        groups.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        groups
    }

    /// The phrases that are left out and have no record in the map.
    fn left_out_phrases(&self) -> BTreeSet<&str> {
        self.left_out
            .iter()
            .map(|l| l.phrase.as_str())
            .filter(|p| !self.mapped_phrases.contains(*p))
            .collect()
    }
}

/// What changed from one report to the next, by phrase. Each list is in
/// sorted order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReportDifference {
    /// Phrases that only the new profile has.
    pub added: Vec<String>,
    /// Phrases that only the old profile has.
    pub removed: Vec<String>,
    /// Phrases that were in the map and are now left out.
    pub moved_to_left_out: Vec<String>,
    /// Phrases that were left out and are now in the map.
    pub moved_to_map: Vec<String>,
}

impl ReportDifference {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty()
            && self.removed.is_empty()
            && self.moved_to_left_out.is_empty()
            && self.moved_to_map.is_empty()
    }
}

/// What changed from `old` to `new`: phrases added, removed, moved to
/// left-out, moved to the map.
pub fn report_difference(old: &BuildReport, new: &BuildReport) -> ReportDifference {
    let old_map: BTreeSet<&str> = old.mapped_phrases.iter().map(String::as_str).collect();
    let new_map: BTreeSet<&str> = new.mapped_phrases.iter().map(String::as_str).collect();
    let old_left = old.left_out_phrases();
    let new_left = new.left_out_phrases();
    let old_all: BTreeSet<&str> = old_map.union(&old_left).copied().collect();
    let new_all: BTreeSet<&str> = new_map.union(&new_left).copied().collect();
    let owned = |set: BTreeSet<&str>| set.into_iter().map(str::to_string).collect();
    ReportDifference {
        added: owned(new_all.difference(&old_all).copied().collect()),
        removed: owned(old_all.difference(&new_all).copied().collect()),
        moved_to_left_out: owned(old_map.intersection(&new_left).copied().collect()),
        moved_to_map: owned(old_left.intersection(&new_map).copied().collect()),
    }
}
