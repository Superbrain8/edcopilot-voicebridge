//! Build the command map from the bytes of a VoiceAttack profile (`.vap`).
//!
//! The library part is pure: bytes in, a map or a typed error out. It reads
//! no file and writes no file. The steps are:
//!
//! 1. Inflate the raw DEFLATE stream ([`record::inflate`]).
//! 2. Walk the record tree and check the structure ([`record::walk`]).
//!    A record with fewer fields than the rules read stops the build. A
//!    record with more fields than the tested format gives a note.
//! 3. Apply the extraction rules of the first map extractor (a Python
//!    script) and the whitelist of `docs/PROFILE_FORMAT.md`. A command
//!    that the builder does not fully understand is left out with a
//!    reason.
//! 4. Check the totals. A result with too few records stops the build.
//!
//! A wrong match is worse than a miss: each doubt about one command leaves
//! that command out.

mod allow;
#[cfg(test)]
mod bound_tests;
#[cfg(test)]
mod build_tests;
mod error;
mod extract;
#[cfg(any(test, feature = "test-support"))]
pub mod fixture;
/// The walk of the record tree. Public for the example `profile_build`,
/// which times the inflate step and shows what the walk reads.
pub mod record;
mod report;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

use std::collections::HashMap;

use sha2::{Digest, Sha256};

pub use error::{BuildError, RecordKind, Totals};
pub use record::{ObservedCounts, OTHER_TYPES};
pub use report::{
    report_difference, BlockingPhrase, BuildReport, FormatNote, LeftOut, LeftOutClass,
    ReportDifference, SourceInfo,
};

use crate::command_map::{Command, CommandClass, CommandType};

/// The smallest number of records that a real profile gives. The tested
/// profile gives about 3,000.
const MIN_RECORDS: usize = 200;

/// A field where at least this many actions (or commands) have another
/// value is a change of the format, when they are also more than the share
/// of the rule. The values can differ.
const FORMAT_CHANGE_ACTIONS: usize = 50;

/// The share (percent) of the actions of the profile, or of the actions of
/// one type, with another value in a field of `record::SHARED_FIELDS`. The
/// same share of the commands for a field of `record::COMMAND_SETTINGS`.
const FORMAT_CHANGE_PERCENT: usize = 5;

/// The share (percent) of the actions of one modelled type with another
/// value in one of the tested fields of that type
/// (`record::TESTED_SETTINGS`).
const FORMAT_CHANGE_TYPE_PERCENT: usize = 20;

/// The share (percent) of the commands with a request write that one
/// reason of the format kinds (see `extract::format_reason`) can leave out.
const FORMAT_REASON_PERCENT: usize = 25;

/// True when `records` of `total` is at least [`FORMAT_CHANGE_ACTIONS`] and
/// more than `percent` percent.
fn many(records: usize, total: usize, percent: usize) -> bool {
    records >= FORMAT_CHANGE_ACTIONS && records * 100 > total * percent
}

/// A change of the format: a field where many actions or many commands
/// have another value, with any values. The rules, in this order:
///
/// 1. a field of `record::SHARED_FIELDS`, over all actions (5 percent);
/// 2. a field of `record::SHARED_FIELDS`, over the actions of one type (5
///    percent of that type);
/// 3. a tested field of a modelled action type, over the actions of that
///    type (20 percent of that type);
/// 4. a field of `record::COMMAND_SETTINGS`, over all commands (5
///    percent).
///
/// Each rule needs at least [`FORMAT_CHANGE_ACTIONS`] records. Fewer
/// records with another value are settings of single commands, which the
/// whitelist leaves out.
fn check_format(profile: &record::RawProfile) -> Result<(), BuildError> {
    let settings = &profile.settings;
    let total: usize = settings.actions_by_type.values().sum();
    let mut by_field: std::collections::BTreeMap<usize, usize> = Default::default();
    for (&(field, _), &actions) in &settings.shared {
        *by_field.entry(field).or_default() += actions;
    }
    for (field, records) in by_field {
        if many(records, total, FORMAT_CHANGE_PERCENT) {
            return Err(BuildError::FormatChanged {
                kind: RecordKind::Action,
                action_type: None,
                field,
                records,
                total,
            });
        }
    }
    for shared in [true, false] {
        for (&(kind, field), &records) in &settings.by_type_field {
            if record::SHARED_FIELDS.contains(&field) != shared {
                continue;
            }
            let percent = if shared {
                FORMAT_CHANGE_PERCENT
            } else {
                FORMAT_CHANGE_TYPE_PERCENT
            };
            let total = settings.actions_by_type.get(&kind).copied().unwrap_or(0);
            if many(records, total, percent) {
                return Err(BuildError::FormatChanged {
                    kind: RecordKind::Action,
                    action_type: Some(kind),
                    field,
                    records,
                    total,
                });
            }
        }
    }
    let total = profile.commands.len();
    for (&field, &records) in &settings.commands {
        if many(records, total, FORMAT_CHANGE_PERCENT) {
            return Err(BuildError::FormatChanged {
                kind: RecordKind::Command,
                action_type: None,
                field,
                records,
                total,
            });
        }
    }
    Ok(())
}

/// One reason of the format kinds (see `extract::format_reason`) that
/// leaves out more than [`FORMAT_REASON_PERCENT`] percent (and at least
/// [`FORMAT_CHANGE_ACTIONS`]) of the commands with a request write: the
/// format changed. `writers`: the commands with a request write.
fn check_reason_share(left_out: &[LeftOut], writers: usize) -> Result<(), BuildError> {
    let mut by_reason: std::collections::BTreeMap<&str, usize> = Default::default();
    for l in left_out {
        if l.class == LeftOutClass::NotUnderstood && extract::format_reason(&l.reason) {
            *by_reason.entry(l.reason.as_str()).or_default() += 1;
        }
    }
    let largest = by_reason
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(a.0)));
    match largest {
        Some((reason, commands)) if many(commands, writers, FORMAT_REASON_PERCENT) => {
            Err(BuildError::ReasonShare {
                reason: reason.to_string(),
                commands,
                total: writers,
            })
        }
        _ => Ok(()),
    }
}

/// The result of a build.
#[derive(Debug, Clone)]
pub struct BuiltMap {
    /// The map records, in profile order.
    pub commands: Vec<Command>,
    /// The profile variables: variable name to grammar.
    pub profile_vars: HashMap<String, String>,
    /// The facts, the format note, and the left-out commands.
    pub report: BuildReport,
    /// The phrases of the left-out commands that VoiceAttack recognises by
    /// voice, in profile order. See [`BlockingPhrase`]. The phrases still
    /// have their `{TXT:EDCP_...}` and profile variables.
    pub blocking: Vec<BlockingPhrase>,
}

/// The reason of the largest group of left-out commands, with its count.
/// Groups of the same size: the first reason in text order.
fn largest_reason(left_out: &[LeftOut]) -> Option<(String, usize)> {
    let mut by_reason: std::collections::BTreeMap<&str, usize> = Default::default();
    for l in left_out {
        *by_reason.entry(l.reason.as_str()).or_default() += 1;
    }
    by_reason
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(a.0)))
        .map(|(reason, n)| (reason.to_string(), n))
}

/// The SHA-256 of the bytes of a `.vap` file, in lowercase hex. It is the
/// hash in `SourceInfo::sha256`.
pub fn source_sha256(vap: &[u8]) -> String {
    Sha256::digest(vap)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Builds the map from the bytes of a `.vap` file.
pub fn build_map(vap: &[u8]) -> Result<BuiltMap, BuildError> {
    build_map_reviewed(vap, &allow::REVIEWED_CODE)
}

/// [`build_map`] with another table of reviewed code hashes. The tests use
/// it with the hash of invented code.
fn build_map_reviewed(vap: &[u8], reviewed: &[&str]) -> Result<BuiltMap, BuildError> {
    let inflated = record::inflate(vap)?;
    let profile = record::walk(&inflated)?;
    drop(inflated);
    let format = FormatNote::from_counts(&profile.counts);
    if matches!(
        format,
        FormatNote::OlderThanTested(_) | FormatNote::Mixed(_)
    ) {
        return Err(BuildError::OlderFormat);
    }
    check_format(&profile)?;
    let out = extract::extract(&profile, reviewed)?;
    if out.writers == 0 {
        return Err(BuildError::Totals(Totals::NoRequestWrite));
    }
    check_reason_share(&out.left_out, out.writers)?;
    let records = out.commands.len();
    if records < MIN_RECORDS {
        let largest_reason = largest_reason(&out.left_out);
        return Err(BuildError::Totals(Totals::TooFewRecords {
            found: records,
            minimum: MIN_RECORDS,
            largest_reason,
        }));
    }
    let fixed = out.commands.iter().any(|c| {
        c.command_type != CommandType::Prefix
            && c.class == CommandClass::Fixed
            && !c.token_template.is_empty()
    });
    if !fixed {
        return Err(BuildError::Totals(Totals::NoFixedRecord));
    }
    let actions = profile.commands.iter().map(|c| c.actions.len()).sum();
    let request_writes = profile
        .commands
        .iter()
        .flat_map(|c| &c.actions)
        .filter(|a| extract::is_write(a))
        .count();
    let sha256 = source_sha256(vap);
    let report = BuildReport {
        source: SourceInfo {
            sha256,
            compressed_len: vap.len(),
            inflated_len: profile.inflated_len,
            voiceattack_version: profile.voiceattack_version.clone(),
            actions,
            request_writes,
        },
        format,
        commands_in_profile: profile.commands.len(),
        records,
        mapped_phrases: out
            .commands
            .iter()
            .filter_map(|c| c.phrase.clone())
            .collect(),
        left_out: out.left_out,
        unknown_settings: report::report_settings(&profile.settings.by_type_field),
        unknown_phrase_vars: out.unknown_phrase_vars,
        cannot_fire: out.cannot_fire,
    };
    Ok(BuiltMap {
        commands: out.commands,
        profile_vars: out.profile_vars,
        report,
        blocking: out.blocking,
    })
}
