//! Inflate a VoiceAttack profile and walk its record tree.
//!
//! The inflated profile is a tree of records. Every record has this form:
//!
//! ```text
//! [i32 size] [i32 n] [i32 offset x (n + 1)] [field 0] ... [field n]
//! ```
//!
//! The offsets count from the record start. Field `k` runs from offset `k`
//! to offset `k + 1`. The last field runs to the record end. Thus a record
//! with the header count `n` has `n + 1` fields. A string field is
//! `[i32 length][UTF-8]`, or the length -1 for a null string. A list field is
//! `[i32 count]` and then `count` records, each with its size first.

use std::collections::{BTreeMap, BTreeSet};

use flate2::{Decompress, FlushDecompress, Status};
use sha2::{Digest, Sha256};

use super::error::{BuildError, RecordKind};

/// The largest inflated profile that the builder accepts. The tested
/// profile inflates to about 9.3 MiB.
pub const MAX_INFLATED: usize = 64 * 1024 * 1024;

/// The first buffer of the inflate step. The buffer grows as the data
/// comes, so a large input does not reserve a large buffer at the start.
const FIRST_BUFFER: usize = 16 * 1024 * 1024;

/// The limits of the walk. Each limit is above the tested profile (3,058
/// commands, 19,569 actions, at most 103 actions in one command), and
/// keeps the memory and the work of a crafted file small.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Limits {
    /// The largest header count of a record (tested: 89, 82, 32).
    pub fields: u32,
    /// The most commands in a profile (tested: about 3,000).
    pub commands: usize,
    /// The most actions in a profile (tested: about 20,000).
    pub actions: usize,
    /// The most actions in one command.
    pub actions_per_command: usize,
}

/// The limits of [`walk`].
pub(crate) const LIMITS: Limits = Limits {
    fields: 1024,
    commands: 20_000,
    actions: 200_000,
    actions_per_command: 2_000,
};

/// A header field count for each record kind. A record with the header
/// count `n` has the fields `0..=n`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldCounts {
    pub profile: u32,
    pub command: u32,
    pub action: u32,
}

/// The header field counts of the tested profile format (VoiceAttack
/// profile version "1.16", EDCoPilot 1.11).
pub const TESTED_FIELD_COUNTS: FieldCounts = FieldCounts {
    profile: 89,
    command: 82,
    action: 32,
};

/// The smallest header count that the walk reads: the largest field index
/// that a rule reads, for each record kind. Profile: field 32 (the
/// VoiceAttack version). Command: field 20 (the composite group). Action:
/// field 30 (the compound condition). A count below the tested count is an
/// older format, which the builder refuses after the walk. A count above
/// it is accepted with a note, because each new VoiceAttack version so far
/// added its fields at the end of the records.
pub const MIN_FIELDS: FieldCounts = FieldCounts {
    profile: 32,
    command: 20,
    action: 30,
};

/// The length signature of the tested format: (field, length in bytes) of
/// each field that has one length in each record of its kind. A field that
/// is a null string in each record is not in it: another profile can give
/// it a text. A record with another length in one of these fields has a
/// moved or changed field, and the builder stops.
pub const PROFILE_SIGNATURE: [(usize, usize); 13] = [
    (0, 16),
    (3, 1),
    (5, 1),
    (6, 2),
    (11, 1),
    (12, 1),
    (14, 1),
    (15, 1),
    (16, 2),
    (21, 1),
    (22, 1),
    (23, 1),
    (25, 1),
];
/// The command fields of [`COMMAND_SIGNATURE`] that each command needs:
/// the id and the command type. Another length stops the build. Another
/// length of each other field of the signature is a setting of one command:
/// the builder leaves that command out.
const COMMAND_IDENTITY: [(usize, usize); 2] = [(0, 16), (19, 4)];
/// See [`PROFILE_SIGNATURE`].
pub const COMMAND_SIGNATURE: [(usize, usize); 15] = [
    (0, 16),
    (3, 1),
    (4, 1),
    (8, 1),
    (9, 4),
    (10, 4),
    (11, 4),
    (12, 4),
    (13, 4),
    (14, 2),
    (15, 2),
    (16, 1),
    (17, 4),
    (18, 4),
    (19, 4),
];
/// See [`PROFILE_SIGNATURE`].
pub const ACTION_SIGNATURE: [(usize, usize); 21] = [
    (0, 16),
    (1, 4),
    (2, 8),
    (3, 8),
    (9, 4),
    (10, 4),
    (11, 4),
    (12, 4),
    (13, 4),
    (16, 4),
    (17, 4),
    (19, 4),
    (20, 4),
    (21, 4),
    (23, 4),
    (24, 16),
    (25, 16),
    (26, 12),
    (27, 12),
    (28, 1),
    (29, 4),
];

/// The condition operators that the tested profile uses (action field 19
/// of the action types 19 and 63). For a text compare (field 23 = 1), 6 is
/// "contains" and 9 is "has not been set". The rules use only these two.
pub const KNOWN_OPERATORS: [i32; 7] = [0, 1, 2, 3, 4, 6, 9];

/// Action type: begin a condition.
pub const COND_BEGIN: i32 = 19;
/// Action type: end a condition.
pub const COND_END: i32 = 20;
/// Action type: else.
pub const COND_ELSE: i32 = 29;
/// Action type: else if.
pub const COND_ELSE_IF: i32 = 63;
/// Action type: execute another command.
pub const EXECUTE: i32 = 16;
/// Action type: set a text variable.
pub const SET_TEXT: i32 = 21;
/// Action type: run a plugin function.
pub const PLUGIN: i32 = 22;
/// Action type: set an integer variable.
pub const SET_INT: i32 = 37;
/// Action type: write a text to a file.
pub const WRITE_FILE: i32 = 44;
/// Action type: run inline C# code.
pub const INLINE_CODE: i32 = 71;
/// Action type: press keys.
pub const KEY_PRESS: i32 = 0;
/// Action type: set a small integer.
pub const SET_SMALL_INT: i32 = 18;
/// Action type: set the clipboard.
pub const CLIPBOARD: i32 = 24;
/// Action type: begin a loop.
pub const LOOP_BEGIN: i32 = 30;
/// Action type: end a loop.
pub const LOOP_END: i32 = 31;
/// Action type: set a boolean.
pub const SET_BOOL: i32 = 36;
/// Action type: set a decimal.
pub const SET_DECIMAL: i32 = 38;
/// Action type: wait for a spoken response.
pub const WAIT_FOR_SPEECH: i32 = 74;
/// Action type: set a text from an integer.
pub const SET_TEXT_FROM_INT: i32 = 77;

const ZERO4: &[u8] = &[0; 4];
const NULL4: &[u8] = &[0xff; 4];
const ZERO8: &[u8] = &[0; 8];
const ZERO16: &[u8] = &[0; 16];
/// The value of the fields 26 and 27 in each action of the tested profile.
const STAMP: &[u8] = &[0x00, 0x09, 0x6e, 0x88, 0xf1, 0xff, 0xff, 0xff, 0, 0, 0, 0];

/// The value of an action field that the builder does not read, as each
/// action of a modelled type has it in the tested profile.
pub(crate) fn tested_default(field: usize) -> &'static [u8] {
    match field {
        2 | 3 => ZERO8,
        5..=9 | 14 | 15 | 18 | 22 => NULL4,
        24 | 25 => ZERO16,
        26 | 27 => STAMP,
        _ => ZERO4,
    }
}

/// The action fields with one value in each action of the tested profile,
/// of each type. A different value that many actions share is a change of
/// the format, not a setting of the author (see `BuildError::FormatChanged`).
pub const SHARED_FIELDS: [usize; 6] = [25, 26, 27, 29, 31, 32];

/// The string fields of `tested_default`: a null string (the tested value)
/// and an empty string mean the same. Each field has strings of different
/// lengths in the tested profile. Field 9 is not in the list: it has 4
/// bytes in each action (the integers -1 and 0).
const STRING_FIELDS: [usize; 8] = [5, 6, 7, 8, 14, 15, 18, 22];

/// The list field of `tested_default`: an empty list (the tested value)
/// and a null list mean the same.
const LIST_FIELD: usize = 30;

/// The field `field` has the tested value. A null string is equal to an
/// empty string, and a null list is equal to an empty list.
pub(crate) fn is_tested(field: usize, bytes: &[u8]) -> bool {
    bytes == tested_default(field)
        || (STRING_FIELDS.contains(&field) && bytes == ZERO4)
        || (field == LIST_FIELD && bytes == NULL4)
}

/// For each action type that the builder models: the fields that it does
/// not read and that have one value (see [`tested_default`]) in each such
/// action of the tested profile. The fields that vary are in `docs/PROFILE_FORMAT.md`.
/// Write field 11 is 1 only on 8 writes to another file (probably
/// "append"), and the builder requires 0.
pub(crate) const TESTED_SETTINGS: [(i32, &[usize]); 8] = [
    (
        SET_TEXT,
        &[
            2, 3, 4, 9, 10, 11, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 29, 30, 31, 32,
        ],
    ),
    (
        WRITE_FILE,
        &[
            2, 3, 4, 7, 8, 9, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27,
            29, 30, 31, 32,
        ],
    ),
    (
        COND_BEGIN,
        &[
            3, 4, 7, 8, 9, 10, 11, 13, 14, 15, 24, 25, 26, 27, 29, 31, 32,
        ],
    ),
    (
        COND_ELSE_IF,
        &[
            2, 3, 4, 7, 8, 9, 10, 11, 13, 14, 15, 24, 25, 26, 27, 29, 31, 32,
        ],
    ),
    (
        COND_END,
        &[
            2, 3, 4, 5, 6, 7, 8, 9, 11, 12, 13, 14, 15, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 29,
            30, 31, 32,
        ],
    ),
    (
        COND_ELSE,
        &[
            2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27,
            29, 30, 31, 32,
        ],
    ),
    (
        EXECUTE,
        &[
            2, 4, 7, 8, 9, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 29,
            30, 31, 32,
        ],
    ),
    (
        SET_INT,
        &[
            2, 3, 4, 5, 6, 7, 8, 9, 12, 16, 17, 18, 20, 21, 22, 23, 24, 25, 26, 27, 29, 30, 31, 32,
        ],
    ),
];

/// One action of a command, with the fields that the rules use.
///
/// The walk reads a field only for the action types whose rules read it.
/// A field that the walk does not read is `None` (or `0`, or `false`). A
/// field that it reads must have the correct length and encoding.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RawAction {
    /// The byte position of the action record.
    pub offset: usize,
    /// Field 1: the action type. Read for each action.
    pub kind: i32,
    /// Field 5. Set text: the variable name. Write to a file: the text.
    /// Execute a command: the id of the command, as GUID text. Inline
    /// code: the code text. Plugin: the plugin id.
    pub text5: Option<String>,
    /// Field 6. Set text: the value. Write to a file: the path. Condition:
    /// the compare value. Execute a command: the name of the command.
    pub text6: Option<String>,
    /// Field 7. Set text: the variable to copy.
    pub text7: Option<String>,
    /// Field 8. Set text: the file to read. Plugin: the context.
    pub text8: Option<String>,
    /// Field 12. Set text: 1 replaces a text in the value. Condition: 1 on
    /// each `{PREFIX}` block of the tested profile.
    pub int12: i32,
    /// Field 13. Set text: the source (0 is the literal in field 6). Set
    /// an integer: the operation.
    pub int13: i32,
    /// Field 14. Set text: the text to replace. Set an integer: the
    /// variable.
    pub text14: Option<String>,
    /// Field 15. Set text: the replacement. Set an integer: the value.
    pub text15: Option<String>,
    /// Field 18. Condition: the variable.
    pub variable: Option<String>,
    /// Field 19. Condition: the operator.
    pub operator: i32,
    /// Field 20. Condition: not 0 in a few compares of another kind.
    pub int20: i32,
    /// Field 21. Condition: not 0 for a compare with a second variable.
    pub int21: i32,
    /// Field 22. Condition: the second variable.
    pub text22: Option<String>,
    /// Field 23. Condition: the value kind (1 is text).
    pub int23: i32,
    /// Field 30 is not the empty list. Condition: a compound condition.
    pub compound: bool,
    /// Field 3 of an execute action: `Some(true)` for the mode "by name"
    /// (the double 1.0), `Some(false)` for "by id" (0.0), `None` for
    /// another value.
    pub exec_by_name: Option<bool>,
    /// Inline code: the SHA-256 of the fields 5 (code), 6 (assemblies) and
    /// 11, each with its length first.
    pub code_hash: Option<[u8; 32]>,
    /// The first field of a modelled action type that has another value
    /// than in the tested profile (see `TESTED_SETTINGS`).
    pub unknown_setting: Option<usize>,
    /// Field 28 is 1. Probably "the action is disabled" (see `docs/PROFILE_FORMAT.md`).
    /// Read for each action.
    pub flag28: bool,
    /// A set action (types 18, 21, 36, 37, 38, 74, 77): the name of the
    /// variable that it sets. `None` for another type, or when the field
    /// is not a string.
    pub set_name: Option<String>,
    /// A set action whose variable name has a `{` (VoiceAttack puts the
    /// value of a token there, so the action can set any variable), or
    /// whose name field is not a string.
    pub token_name: bool,
    /// Field 2 of a condition (types 19 and 63) is the double 0.0. The
    /// tested profile has 0.0 in each condition of a suffix.
    pub field2_zero: bool,
    /// Set an integer (type 37): the fields 10, 11 and 19. They vary in the
    /// tested profile. The system-slot parse has fixed values.
    pub int10: i32,
    pub int11: i32,
    pub int19: i32,
    /// A newer format: the fields after the tested field count of an
    /// action of a modelled type, with their index. The walk compares them
    /// with the most common value of each field of the type, and then
    /// clears the list.
    pub appended: Vec<(usize, Vec<u8>)>,
}

/// The field of the variable name of each set action type.
pub const SET_NAME_FIELDS: [(i32, usize); 7] = [
    (SET_TEXT, 5),
    (SET_BOOL, 5),
    (SET_SMALL_INT, 14),
    (SET_INT, 14),
    (SET_DECIMAL, 14),
    (WAIT_FOR_SPEECH, 6),
    (SET_TEXT_FROM_INT, 6),
];

/// One command of the profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawCommand {
    /// The byte position of the command record.
    pub offset: usize,
    /// Field 0: the command id (16 bytes).
    pub id: [u8; 16],
    /// Field 1: the phrase.
    pub phrase: Option<String>,
    /// Field 7: the category.
    pub category: Option<String>,
    /// Field 19: 0 full, 1 prefix, 2 suffix. The rules leave out a
    /// command with another value.
    pub command_type: i32,
    /// Field 20: the composite group.
    pub group: Option<String>,
    /// Field 4 is the byte 1 (probably "enabled"; 1 in each command of the
    /// tested profile). Also true when field 4 has another length.
    pub enabled: bool,
    /// Field 15 is the bytes 1, 1 (probably "when I say"; the 7
    /// subroutine commands of the tested profile have 1, 0). Also true when
    /// field 15 has another length.
    pub voice_trigger: bool,
    /// Field 2: the actions, in order.
    pub actions: Vec<RawAction>,
    /// The first field of [`COMMAND_SETTINGS`] with another value than in
    /// the tested profile. VoiceAttack has command options (for example a
    /// repeat), and the builder does not know which field holds which.
    pub unknown_setting: Option<usize>,
}

/// The command fields that have one value in each command of the tested
/// profile (all 3,058 commands), with that value, in field order. A field
/// with another length or value is another setting. Fields 9, 22, 32, 37,
/// 74 and 76 have more than one value and are not in the list.
pub const COMMAND_SETTINGS: [(usize, &[u8]); 62] = [
    (5, NULL4),
    (8, &[0]),
    (10, ZERO4),
    (11, ZERO4),
    (12, ZERO4),
    (13, ZERO4),
    (16, &[0]),
    (17, &[2, 0, 0, 0]),
    (18, ZERO4),
    (23, ZERO4),
    (24, &[0]),
    (25, ZERO4),
    (26, ZERO4),
    (27, ZERO4),
    (28, ZERO4),
    (29, &[0]),
    (30, &[0]),
    (31, &[0]),
    (34, &[0]),
    (35, &[0]),
    (36, &[1]),
    (38, &[0]),
    (39, &[0]),
    (40, &[0]),
    (41, &[0]),
    (42, &[0]),
    (43, &[0]),
    (44, &[0]),
    (45, &[0]),
    (46, &[0]),
    (47, &[0]),
    (48, &[1, 1]),
    (49, &[0]),
    (51, &[0]),
    (52, NULL4),
    (53, &[0]),
    (54, &[0]),
    (55, &[0]),
    (56, &[0]),
    (57, ZERO4),
    (58, &[1, 0, 0, 0]),
    (59, &[0]),
    (60, &[0]),
    (61, NULL4),
    (62, NULL4),
    (63, ZERO4),
    (64, &[0]),
    (65, &[0]),
    (66, &[0; 17]),
    (68, ZERO4),
    (69, ZERO4),
    (70, ZERO4),
    (71, ZERO4),
    (72, ZERO4),
    (73, ZERO4),
    (75, ZERO4),
    (77, &[0]),
    (78, &[0]),
    (79, &[0]),
    (80, NULL4),
    (81, &[0]),
    (82, NULL4),
];

/// The fields of [`COMMAND_SETTINGS`] that hold a null string in each
/// command of the tested profile. An empty string is the same setting.
pub const COMMAND_STRING_FIELDS: [usize; 6] = [5, 52, 61, 62, 80, 82];

/// The command field `field` of [`COMMAND_SETTINGS`] has its tested value
/// `value`. A null string is equal to an empty string.
fn is_tested_command(field: usize, value: &[u8], bytes: &[u8]) -> bool {
    bytes == value || (COMMAND_STRING_FIELDS.contains(&field) && bytes == ZERO4)
}

/// The header field counts that the walk saw.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ObservedCounts {
    pub profile: i64,
    pub command: BTreeSet<i64>,
    pub action: BTreeSet<i64>,
}

/// The walked profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawProfile {
    pub inflated_len: usize,
    pub counts: ObservedCounts,
    /// Profile field 32: the VoiceAttack version text (digits and dots).
    pub voiceattack_version: String,
    pub commands: Vec<RawCommand>,
    /// The settings of the actions that differ from the tested profile.
    pub settings: SettingTally,
}

/// The most action types with their own counts in the [`SettingTally`].
/// The tested profile has 35 types.
pub(crate) const MAX_TALLY_TYPES: usize = 256;

/// The type key of the actions of the types after the first
/// [`MAX_TALLY_TYPES`] types in the [`SettingTally`].
pub const OTHER_TYPES: i32 = i32::MIN;

/// The action settings that differ from the tested profile.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SettingTally {
    /// The number of actions with another value, by (action type, field),
    /// for the fields of `TESTED_SETTINGS` and of [`SHARED_FIELDS`].
    pub by_type_field: BTreeMap<(i32, usize), usize>,
    /// The number of actions of any type with another value in a field of
    /// [`SHARED_FIELDS`], by (field, value).
    pub shared: BTreeMap<(usize, Vec<u8>), usize>,
    /// The number of commands with another value in a field of
    /// [`COMMAND_SETTINGS`], by field.
    pub commands: BTreeMap<usize, usize>,
    /// The number of actions of each type.
    pub actions_by_type: BTreeMap<i32, usize>,
}

/// Inflates one raw DEFLATE stream. The stream must use all input bytes.
pub fn inflate(input: &[u8]) -> Result<Vec<u8>, BuildError> {
    let mut d = Decompress::new(false);
    let mut out: Vec<u8> = Vec::with_capacity(input.len().saturating_mul(10).min(FIRST_BUFFER));
    loop {
        if out.len() == out.capacity() {
            if out.len() >= MAX_INFLATED {
                return Err(BuildError::TooLarge {
                    limit: MAX_INFLATED,
                });
            }
            let more = out.capacity().max(64 * 1024).min(MAX_INFLATED - out.len());
            out.reserve_exact(more);
        }
        let used = d.total_in() as usize;
        let produced = out.len();
        let status = d
            .decompress_vec(&input[used..], &mut out, FlushDecompress::None)
            .map_err(|_| BuildError::Inflate {
                offset: d.total_in() as usize,
            })?;
        if status == Status::StreamEnd {
            break;
        }
        let no_progress = d.total_in() as usize == used && out.len() == produced;
        if no_progress && out.len() < out.capacity() {
            return Err(BuildError::Truncated);
        }
    }
    let used = d.total_in() as usize;
    if used != input.len() {
        return Err(BuildError::TrailingBytes {
            used,
            total: input.len(),
        });
    }
    Ok(out)
}

/// The absolute byte spans of the fields of one record.
struct Record {
    offset: usize,
    fields: Vec<(usize, usize)>,
}

struct Reader<'a> {
    data: &'a [u8],
    limits: Limits,
}

/// `a + b`, or an out-of-bounds error.
fn add(a: usize, b: usize, offset: usize, what: &'static str) -> Result<usize, BuildError> {
    a.checked_add(b)
        .ok_or(BuildError::OutOfBounds { offset, what })
}

/// A version text: it starts with digits, a dot, and digits. Other text
/// can follow.
fn is_version(text: &str) -> bool {
    let digits = |s: &str| s.bytes().take_while(u8::is_ascii_digit).count();
    let major = digits(text);
    major > 0 && text[major..].starts_with('.') && digits(&text[major + 1..]) > 0
}

impl<'a> Reader<'a> {
    fn i32_at(&self, at: usize, what: &'static str) -> Result<i32, BuildError> {
        let end = add(at, 4, at, what)?;
        let bytes = self
            .data
            .get(at..end)
            .ok_or(BuildError::OutOfBounds { offset: at, what })?;
        Ok(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// Reads the record at `start`, which must end at or before `end`.
    /// `minimum`: the smallest header count that the rules accept.
    fn record(
        &self,
        start: usize,
        end: usize,
        kind: RecordKind,
        minimum: u32,
    ) -> Result<(Record, i64), BuildError> {
        let out = |what| BuildError::OutOfBounds {
            offset: start,
            what,
        };
        if add(start, 8, start, "a record header")? > end {
            return Err(out("a record header"));
        }
        let size = self.i32_at(start, "a record size")?;
        if size < 8 || add(start, size as usize, start, "a record")? > end {
            return Err(out("a record"));
        }
        let size = size as usize;
        let n = self.i32_at(start + 4, "a field count")? as i64;
        if n < minimum as i64 {
            return Err(BuildError::FieldCount {
                kind,
                offset: start,
                minimum,
                found: n,
            });
        }
        if n > self.limits.fields as i64 {
            return Err(BuildError::TooManyFields {
                kind,
                offset: start,
                maximum: self.limits.fields,
                found: n,
            });
        }
        // `n` is at most `limits.fields`: no overflow.
        let table_end = 8 + 4 * (n as usize + 1);
        if table_end > size {
            return Err(out("an offset table"));
        }
        let mut offsets = Vec::with_capacity(n as usize + 2);
        for k in 0..=n as usize {
            offsets.push(self.i32_at(start + 8 + 4 * k, "an offset")? as i64);
        }
        let monotonic = offsets.windows(2).all(|w| w[0] <= w[1]);
        if offsets[0] != table_end as i64 || !monotonic || offsets[n as usize] > size as i64 {
            return Err(BuildError::BadOffsets { offset: start });
        }
        offsets.push(size as i64);
        // Each offset is from `table_end` to `size`: inside the record.
        let fields = offsets
            .windows(2)
            .map(|w| (start + w[0] as usize, start + w[1] as usize))
            .collect();
        Ok((
            Record {
                offset: start,
                fields,
            },
            n,
        ))
    }

    /// Each field of `signature` has its tested length.
    fn signature(
        &self,
        rec: &Record,
        kind: RecordKind,
        signature: &[(usize, usize)],
    ) -> Result<(), BuildError> {
        for &(field, length) in signature {
            let (a, b) = self.field(rec, field)?;
            if b - a != length {
                return Err(BuildError::Layout {
                    kind,
                    offset: rec.offset,
                    field,
                    length,
                });
            }
        }
        Ok(())
    }

    fn field(&self, rec: &Record, k: usize) -> Result<(usize, usize), BuildError> {
        rec.fields.get(k).copied().ok_or(BuildError::BadField {
            offset: rec.offset,
            field: k,
            expected: "present",
        })
    }

    fn bytes(&self, rec: &Record, k: usize) -> Result<&'a [u8], BuildError> {
        let (a, b) = self.field(rec, k)?;
        Ok(&self.data[a..b])
    }

    /// A string field: the text, or `None` for a null string.
    fn string(&self, rec: &Record, k: usize) -> Result<Option<String>, BuildError> {
        let (a, b) = self.field(rec, k)?;
        let bad = BuildError::BadField {
            offset: rec.offset,
            field: k,
            expected: "a string",
        };
        if b - a < 4 {
            return Err(bad);
        }
        let n = self.i32_at(a, "a string length")?;
        if n == -1 && b - a == 4 {
            return Ok(None);
        }
        if n < 0 || b - a - 4 != n as usize {
            return Err(bad);
        }
        String::from_utf8(self.data[a + 4..b].to_vec())
            .map(Some)
            .map_err(|_| BuildError::BadField {
                offset: rec.offset,
                field: k,
                expected: "UTF-8 text",
            })
    }

    fn int(&self, rec: &Record, k: usize) -> Result<i32, BuildError> {
        let (a, b) = self.field(rec, k)?;
        if b - a != 4 {
            return Err(BuildError::BadField {
                offset: rec.offset,
                field: k,
                expected: "an int32",
            });
        }
        self.i32_at(a, "an int32")
    }

    /// The (start, end) of each record in a list field. `max`: the most
    /// items that the walk accepts, with the name of the items.
    fn list(
        &self,
        rec: &Record,
        k: usize,
        max: (usize, &'static str),
    ) -> Result<Vec<(usize, usize)>, BuildError> {
        let (a, b) = self.field(rec, k)?;
        let bad = BuildError::BadList { offset: a };
        if b - a < 4 {
            return Err(bad);
        }
        let count = self.i32_at(a, "a list count")?;
        if count < 0 {
            return Err(bad);
        }
        if count as usize > max.0 {
            return Err(BuildError::TooMany {
                what: max.1,
                limit: max.0,
            });
        }
        // Each item has at least its 8 header bytes. Check the count against
        // the field size before the count sets the size of an allocation.
        if count as usize > (b - a - 4) / 8 {
            return Err(bad);
        }
        let mut items = Vec::with_capacity(count as usize);
        let mut o = a + 4;
        for _ in 0..count {
            if add(o, 4, o, "a list item")? > b {
                return Err(bad);
            }
            let size = self.i32_at(o, "a record size")?;
            let next = add(o, size.max(0) as usize, o, "a list item")?;
            if size < 8 || next > b {
                return Err(bad);
            }
            items.push((o, next));
            o = next;
        }
        if o != b {
            return Err(bad);
        }
        Ok(items)
    }

    /// A field of one byte, 0 or 1.
    fn flag(&self, rec: &Record, k: usize) -> Result<bool, BuildError> {
        match self.bytes(rec, k)? {
            [0] => Ok(false),
            [1] => Ok(true),
            _ => Err(BuildError::BadField {
                offset: rec.offset,
                field: k,
                expected: "one byte, 0 or 1",
            }),
        }
    }

    /// Field 30 of a condition: an empty list (4 zero bytes) or a list of
    /// compound conditions. True when it is not the empty list.
    fn compound(&self, rec: &Record, k: usize) -> Result<bool, BuildError> {
        let bytes = self.bytes(rec, k)?;
        if bytes.len() < 4 {
            return Err(BuildError::BadField {
                offset: rec.offset,
                field: k,
                expected: "a list",
            });
        }
        Ok(bytes != [0, 0, 0, 0])
    }

    /// The fields of `TESTED_SETTINGS` for `kind` with another value than
    /// the tested one, in field order. A field past the end of the record
    /// is skipped: the builder refuses an older format after the walk.
    fn unknown_settings(&self, rec: &Record, kind: i32) -> Vec<usize> {
        let Some((_, fields)) = TESTED_SETTINGS.iter().find(|(k, _)| *k == kind) else {
            return Vec::new();
        };
        fields
            .iter()
            .copied()
            .filter(|&f| {
                rec.fields
                    .get(f)
                    .is_some_and(|&(a, b)| !is_tested(f, &self.data[a..b]))
            })
            .collect()
    }

    /// Counts the settings of one action: each field of `TESTED_SETTINGS`
    /// with another value, and each value of a field of [`SHARED_FIELDS`]
    /// that is not the tested value.
    fn tally(&self, rec: &Record, kind: i32, unknown: &[usize], tally: &mut SettingTally) {
        // The first types each have their own counts. The others count
        // together, so a crafted file with many types keeps the tally small.
        let kind = if tally.actions_by_type.contains_key(&kind)
            || tally.actions_by_type.len() < MAX_TALLY_TYPES
        {
            kind
        } else {
            OTHER_TYPES
        };
        *tally.actions_by_type.entry(kind).or_default() += 1;
        for &f in unknown {
            *tally.by_type_field.entry((kind, f)).or_default() += 1;
        }
        for f in SHARED_FIELDS {
            if let Some(&(a, b)) = rec.fields.get(f) {
                let bytes = &self.data[a..b];
                if !is_tested(f, bytes) {
                    *tally.shared.entry((f, bytes.to_vec())).or_default() += 1;
                    if !unknown.contains(&f) {
                        *tally.by_type_field.entry((kind, f)).or_default() += 1;
                    }
                }
            }
        }
    }

    /// The hash of an inline code action: the fields 5, 6 and 11, each with
    /// its length first, so that no two field splits give the same input.
    fn code_hash(&self, rec: &Record) -> Result<[u8; 32], BuildError> {
        let mut h = Sha256::new();
        for k in [5, 6, 11] {
            let bytes = self.bytes(rec, k)?;
            h.update((bytes.len() as u64).to_le_bytes());
            h.update(bytes);
        }
        Ok(h.finalize().into())
    }

    fn action(
        &self,
        (start, end): (usize, usize),
        counts: &mut ObservedCounts,
        tally: &mut SettingTally,
    ) -> Result<RawAction, BuildError> {
        let (rec, n) = self.record(start, end, RecordKind::Action, MIN_FIELDS.action)?;
        counts.action.insert(n);
        let n = n as usize;
        self.signature(&rec, RecordKind::Action, &ACTION_SIGNATURE)?;
        let kind = self.int(&rec, 1)?;
        let unknown = self.unknown_settings(&rec, kind);
        self.tally(&rec, kind, &unknown, tally);
        let mut a = RawAction {
            offset: start,
            kind,
            flag28: self.flag(&rec, 28)?,
            unknown_setting: unknown.first().copied(),
            ..RawAction::default()
        };
        if matches!(
            kind,
            SET_TEXT | WRITE_FILE | EXECUTE | COND_BEGIN | COND_ELSE_IF | INLINE_CODE | PLUGIN
        ) {
            a.text5 = self.string(&rec, 5)?;
            a.text6 = self.string(&rec, 6)?;
        }
        if kind == SET_TEXT || kind == PLUGIN {
            a.text7 = self.string(&rec, 7)?;
            a.text8 = self.string(&rec, 8)?;
        }
        if kind == SET_TEXT || kind == COND_BEGIN || kind == COND_ELSE_IF {
            a.int12 = self.int(&rec, 12)?;
        }
        if kind == SET_INT {
            a.int10 = self.int(&rec, 10)?;
            a.int11 = self.int(&rec, 11)?;
            a.int19 = self.int(&rec, 19)?;
        }
        let tested = TESTED_FIELD_COUNTS.action as usize;
        if n > tested && TESTED_SETTINGS.iter().any(|(k, _)| *k == kind) {
            for k in tested + 1..=n {
                a.appended.push((k, self.bytes(&rec, k)?.to_vec()));
            }
        }
        if kind == SET_TEXT || kind == SET_INT {
            a.int13 = self.int(&rec, 13)?;
            a.text14 = self.string(&rec, 14)?;
            a.text15 = self.string(&rec, 15)?;
        }
        if kind == COND_BEGIN || kind == COND_ELSE_IF {
            a.field2_zero = self.bytes(&rec, 2)? == 0f64.to_le_bytes();
            a.variable = self.string(&rec, 18)?;
            a.operator = self.int(&rec, 19)?;
            a.int20 = self.int(&rec, 20)?;
            a.int21 = self.int(&rec, 21)?;
            a.text22 = self.string(&rec, 22)?;
            a.int23 = self.int(&rec, 23)?;
            a.compound = self.compound(&rec, 30)?;
        }
        if kind == EXECUTE {
            a.exec_by_name = match self.bytes(&rec, 3)? {
                b if b == 0f64.to_le_bytes() => Some(false),
                b if b == 1f64.to_le_bytes() => Some(true),
                _ => None,
            };
        }
        if let Some(&(_, k)) = SET_NAME_FIELDS.iter().find(|(t, _)| *t == kind) {
            match self.string(&rec, k) {
                Ok(name) => {
                    a.token_name = name.as_deref().is_some_and(|n| n.contains('{'));
                    a.set_name = name;
                }
                Err(_) => a.token_name = true,
            }
        }
        if kind == INLINE_CODE {
            a.code_hash = Some(self.code_hash(&rec)?);
        }
        Ok(a)
    }

    fn command(
        &self,
        (start, end): (usize, usize),
        counts: &mut ObservedCounts,
        tally: &mut SettingTally,
        actions_left: &mut usize,
    ) -> Result<RawCommand, BuildError> {
        let (rec, n) = self.record(start, end, RecordKind::Command, MIN_FIELDS.command)?;
        counts.command.insert(n);
        // The id and the command type are read for each command: another
        // length stops the build. Another length of a field of the other
        // settings is a setting of this command (see `unknown_setting`).
        self.signature(&rec, RecordKind::Command, &COMMAND_IDENTITY)?;
        let odd_length: Vec<usize> = COMMAND_SIGNATURE
            .iter()
            .filter(|(field, length)| {
                self.field(&rec, *field)
                    .is_ok_and(|(a, b)| b - a != *length)
            })
            .map(|(field, _)| *field)
            .collect();
        let id: [u8; 16] = self
            .bytes(&rec, 0)?
            .try_into()
            .map_err(|_| BuildError::BadField {
                offset: start,
                field: 0,
                expected: "a 16-byte id",
            })?;
        let command_type = self.int(&rec, 19)?;
        let spans = self.list(
            &rec,
            2,
            (self.limits.actions_per_command, "actions in one command"),
        )?;
        *actions_left = actions_left
            .checked_sub(spans.len())
            .ok_or(BuildError::TooMany {
                what: "actions",
                limit: self.limits.actions,
            })?;
        let actions = spans
            .into_iter()
            .map(|span| self.action(span, counts, tally))
            .collect::<Result<Vec<_>, _>>()?;
        // A field past the end of the record is skipped: the builder
        // refuses an older format after the walk.
        let mut unknown: Vec<usize> = COMMAND_SETTINGS
            .iter()
            .filter(|(k, value)| {
                self.bytes(&rec, *k)
                    .is_ok_and(|b| !is_tested_command(*k, value, b))
            })
            .map(|(k, _)| *k)
            .chain(odd_length)
            .collect();
        unknown.sort_unstable();
        unknown.dedup();
        for &k in &unknown {
            *tally.commands.entry(k).or_default() += 1;
        }
        Ok(RawCommand {
            offset: start,
            id,
            phrase: self.string(&rec, 1)?,
            category: self.string(&rec, 7)?,
            command_type,
            group: self.string(&rec, 20)?,
            // Fix round 10: a field 4 or 15 with another length cannot be
            // read. Then the command counts as spoken, thus it keeps its
            // blocking phrase. The unknown setting keeps it out of the map.
            enabled: spoken_flag(self.bytes(&rec, 4)?, &[1]),
            voice_trigger: spoken_flag(self.bytes(&rec, 15)?, &[1, 1]),
            unknown_setting: unknown.first().copied(),
            actions,
        })
    }
}

/// The value of the command flag `bytes` (field 4 or 15): true when it is
/// equal to `on`. A flag with another length than `on` cannot be read, and
/// it is also true (the strict direction for the blocking phrases).
fn spoken_flag(bytes: &[u8], on: &[u8]) -> bool {
    bytes.len() != on.len() || bytes == on
}

/// A newer format adds fields at the end of each record. For each action
/// of a modelled type, an appended field with another value than the most
/// common value of that field among the actions of the type is a setting
/// that the program does not know: the action gets `unknown_setting`, and
/// the tally counts it. Equal counts: the smaller bytes are the common
/// value. The lists of the appended fields are cleared.
fn mark_appended(commands: &mut [RawCommand], tally: &mut SettingTally) {
    let mut values: BTreeMap<(i32, usize), BTreeMap<Vec<u8>, usize>> = BTreeMap::new();
    for a in commands.iter().flat_map(|c| &c.actions) {
        for (k, v) in &a.appended {
            *values
                .entry((a.kind, *k))
                .or_default()
                .entry(v.clone())
                .or_default() += 1;
        }
    }
    let common: BTreeMap<(i32, usize), Vec<u8>> = values
        .into_iter()
        .filter_map(|(key, counts)| {
            let top = counts
                .into_iter()
                .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))?;
            Some((key, top.0))
        })
        .collect();
    for a in commands.iter_mut().flat_map(|c| c.actions.iter_mut()) {
        for (k, v) in std::mem::take(&mut a.appended) {
            if common.get(&(a.kind, k)) != Some(&v) {
                a.unknown_setting.get_or_insert(k);
                let kind = if tally.actions_by_type.contains_key(&a.kind) {
                    a.kind
                } else {
                    OTHER_TYPES
                };
                *tally.by_type_field.entry((kind, k)).or_default() += 1;
            }
        }
    }
}

/// Walks the inflated profile. Each record must have at least the field
/// counts of [`MIN_FIELDS`] and the length signature of the tested format.
pub fn walk(data: &[u8]) -> Result<RawProfile, BuildError> {
    walk_with(data, LIMITS)
}

/// [`walk`] with other limits (the tests use small limits).
pub(crate) fn walk_with(data: &[u8], limits: Limits) -> Result<RawProfile, BuildError> {
    let reader = Reader { data, limits };
    let declared = reader
        .i32_at(0, "the root record size")
        .map_err(|_| BuildError::RootSize {
            declared: -1,
            actual: data.len(),
        })?;
    if declared < 0 || declared as usize != data.len() {
        return Err(BuildError::RootSize {
            declared: declared as i64,
            actual: data.len(),
        });
    }
    let (root, n) = reader.record(0, data.len(), RecordKind::Profile, MIN_FIELDS.profile)?;
    reader.signature(&root, RecordKind::Profile, &PROFILE_SIGNATURE)?;
    let mut counts = ObservedCounts {
        profile: n,
        ..ObservedCounts::default()
    };
    let voiceattack_version = match reader.string(&root, 32)? {
        Some(v) if is_version(&v) => v,
        _ => return Err(BuildError::Version),
    };
    let mut actions_left = limits.actions;
    let mut settings = SettingTally::default();
    let commands = reader
        .list(&root, 2, (limits.commands, "commands"))?
        .into_iter()
        .map(|span| reader.command(span, &mut counts, &mut settings, &mut actions_left))
        .collect::<Result<Vec<_>, _>>()?;
    let mut commands = commands;
    mark_appended(&mut commands, &mut settings);
    Ok(RawProfile {
        inflated_len: data.len(),
        counts,
        voiceattack_version,
        commands,
        settings,
    })
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::*;

    #[test]
    fn test_walk_reads_commands_and_actions() {
        let data = profile(vec![
            command("hello", vec![write("Hello"), set_text("v", "1")], 0, None),
            command("bye", vec![], 1, Some("g")),
        ]);
        let p = walk(&data).unwrap();
        assert_eq!(p.commands.len(), 2);
        let c = &p.commands[0];
        assert_eq!(c.phrase.as_deref(), Some("hello"));
        assert_eq!(c.category.as_deref(), Some("cat"));
        assert_eq!(c.actions.len(), 2);
        assert_eq!(c.actions[0].kind, 44);
        assert_eq!(c.actions[0].text5.as_deref(), Some("Hello"));
        assert_eq!(c.actions[1].text6.as_deref(), Some("1"));
        assert_eq!(p.commands[1].command_type, 1);
        assert_eq!(p.commands[1].group.as_deref(), Some("g"));
        assert_eq!(p.counts.profile, 89);
        assert_eq!(p.counts.command, [82].into());
        assert_eq!(p.counts.action, [32].into());
        assert_eq!(p.voiceattack_version, "1.16");
    }

    #[test]
    fn test_type_specific_fields_are_read() {
        let mut cond = action_fields(19);
        cond[18] = str_field("v");
        cond[19] = int_field(9);
        cond[20] = int_field(2);
        cond[21] = int_field(1);
        cond[22] = str_field("w");
        cond[23] = int_field(1);
        cond[30] = [int_field(1), vec![0; 8]].concat();
        let data = profile(vec![command(
            "x",
            vec![
                set_text_replace("a", "b", "c"),
                set_text_from_file("d", "e.txt"),
                record(cond),
            ],
            0,
            None,
        )]);
        let acts = &walk(&data).unwrap().commands[0].actions;
        assert_eq!((acts[0].int12, acts[0].text14.as_deref()), (1, Some("c")));
        assert_eq!(
            (acts[1].int13, acts[1].text8.as_deref()),
            (4, Some("e.txt"))
        );
        let c = &acts[2];
        assert_eq!(c.variable.as_deref(), Some("v"));
        assert_eq!((c.operator, c.int20, c.int21, c.int23), (9, 2, 1, 1));
        assert_eq!(c.text22.as_deref(), Some("w"));
        assert!(c.compound);
    }

    #[test]
    fn test_fields_of_other_types_are_not_read() {
        // Field 5 of a sound action is not a string: the rules do not read it.
        let mut act = action_fields(14);
        act[5] = vec![1, 2];
        let data = profile(vec![command("x", vec![record(act)], 0, None)]);
        assert!(walk(&data).is_ok());
        // The same field of a write is read.
        let mut act = action_fields(44);
        act[5] = vec![1, 2];
        let data = profile(vec![command("x", vec![record(act)], 0, None)]);
        assert!(matches!(
            walk(&data),
            Err(BuildError::BadField { field: 5, .. })
        ));
    }

    #[test]
    fn test_null_and_empty_strings_differ() {
        let mut act = action_fields(21);
        act[5] = null_str();
        act[6] = str_field("");
        let data = profile(vec![command("x", vec![record(act)], 0, None)]);
        let a = &walk(&data).unwrap().commands[0].actions[0];
        assert_eq!(a.text5, None);
        assert_eq!(a.text6.as_deref(), Some(""));
    }

    #[test]
    fn test_flag28_is_read_and_checked() {
        let data = profile(vec![command("x", vec![flagged(sound())], 0, None)]);
        assert!(walk(&data).unwrap().commands[0].actions[0].flag28);
        let act = set_field(sound(), 28, vec![2]);
        let data = profile(vec![command("x", vec![act], 0, None)]);
        assert!(matches!(
            walk(&data),
            Err(BuildError::BadField { field: 28, .. })
        ));
        // Another length is a moved field.
        for bad in [vec![1, 0], vec![]] {
            let act = set_field(sound(), 28, bad);
            let data = profile(vec![command("x", vec![act], 0, None)]);
            assert!(matches!(
                walk(&data),
                Err(BuildError::Layout {
                    kind: RecordKind::Action,
                    field: 28,
                    length: 1,
                    ..
                })
            ));
        }
    }

    #[test]
    fn test_a_field_with_another_length_breaks_the_signature() {
        let mut cmd = command_fields("x", vec![], 0, None);
        cmd[19] = vec![1, 1];
        let data = profile_raw(vec![record(cmd)]);
        assert!(matches!(
            walk(&data),
            Err(BuildError::Layout {
                kind: RecordKind::Command,
                field: 19,
                ..
            })
        ));
        // Another length of a setting field marks the command.
        let mut cmd = command_fields("x", vec![], 0, None);
        cmd[4] = vec![1, 1];
        let p = walk(&profile_raw(vec![record(cmd)])).unwrap();
        assert_eq!(p.commands[0].unknown_setting, Some(4));
        // A field that the walk cannot read counts as switched on.
        assert!(p.commands[0].enabled);
        let act = set_field(sound(), 2, vec![0; 4]);
        let data = profile(vec![command("x", vec![act], 0, None)]);
        assert!(matches!(
            walk(&data),
            Err(BuildError::Layout {
                kind: RecordKind::Action,
                field: 2,
                length: 8,
                ..
            })
        ));
        let mut root = fields_of(&profile(vec![]));
        root[3] = vec![0, 0];
        assert!(matches!(
            walk(&record(root)),
            Err(BuildError::Layout {
                kind: RecordKind::Profile,
                field: 3,
                ..
            })
        ));
    }

    #[test]
    fn test_version_text_must_start_with_a_version_number() {
        for good in ["1.16", "1.10.3", "1.16 beta", "2.0-rc1", "10.20.x"] {
            let mut root = fields_of(&profile(vec![]));
            root[32] = str_field(good);
            assert_eq!(walk(&record(root)).unwrap().voiceattack_version, good);
        }
        for bad in [
            Some("2"),
            Some(""),
            Some("1."),
            Some(".1"),
            Some("v1.16"),
            None,
        ] {
            let mut root = fields_of(&profile(vec![]));
            root[32] = opt_str(bad);
            assert_eq!(walk(&record(root)), Err(BuildError::Version), "{bad:?}");
        }
    }

    #[test]
    fn test_a_setting_with_another_value_is_marked() {
        let read = |act: Vec<u8>| {
            walk(&profile(vec![command("x", vec![act], 0, None)]))
                .unwrap()
                .commands[0]
                .actions[0]
                .unknown_setting
        };
        assert_eq!(read(set_text("a", "b")), None);
        assert_eq!(
            read(set_field(set_text("a", "b"), 4, int_field(1))),
            Some(4)
        );
        assert_eq!(read(set_field(write("A"), 11, int_field(1))), Some(11));
        assert_eq!(read(set_field(write("A"), 26, vec![0; 12])), Some(26));
        // Field 2 of a condition varies in the tested profile: not checked.
        let cond = set_field(condition("v", "x", 6), 2, 1f64.to_le_bytes().to_vec());
        assert_eq!(read(cond), None);
        // A type that the builder does not model: not checked.
        assert_eq!(read(set_field(sound(), 4, int_field(1))), None);
    }

    /// The tested values and the checked fields, as literal text. The test
    /// writer takes its values from `tested_default`. This test does not,
    /// so a wrong value in `tested_default` cannot hide in both.
    #[test]
    fn test_tested_settings_are_pinned() {
        let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
        let expected = |field: usize| match field {
            2 | 3 => "0000000000000000",
            5 | 6 | 7 | 8 | 9 | 14 | 15 | 18 | 22 => "ffffffff",
            24 | 25 => "00000000000000000000000000000000",
            26 | 27 => "00096e88f1ffffff00000000",
            _ => "00000000",
        };
        let fields: [(i32, &[usize]); 8] = [
            (
                21,
                &[
                    2, 3, 4, 9, 10, 11, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 29, 30, 31,
                    32,
                ],
            ),
            (
                44,
                &[
                    2, 3, 4, 7, 8, 9, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25,
                    26, 27, 29, 30, 31, 32,
                ],
            ),
            (
                19,
                &[
                    3, 4, 7, 8, 9, 10, 11, 13, 14, 15, 24, 25, 26, 27, 29, 31, 32,
                ],
            ),
            (
                63,
                &[
                    2, 3, 4, 7, 8, 9, 10, 11, 13, 14, 15, 24, 25, 26, 27, 29, 31, 32,
                ],
            ),
            (
                20,
                &[
                    2, 3, 4, 5, 6, 7, 8, 9, 11, 12, 13, 14, 15, 18, 19, 20, 21, 22, 23, 24, 25, 26,
                    27, 29, 30, 31, 32,
                ],
            ),
            (
                29,
                &[
                    2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 18, 19, 20, 21, 22, 23, 24, 25,
                    26, 27, 29, 30, 31, 32,
                ],
            ),
            (
                16,
                &[
                    2, 4, 7, 8, 9, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26,
                    27, 29, 30, 31, 32,
                ],
            ),
            (
                37,
                &[
                    2, 3, 4, 5, 6, 7, 8, 9, 12, 16, 17, 18, 20, 21, 22, 23, 24, 25, 26, 27, 29, 30,
                    31, 32,
                ],
            ),
        ];
        assert_eq!(TESTED_SETTINGS.len(), fields.len());
        for ((kind, list), (pinned_kind, pinned)) in TESTED_SETTINGS.iter().zip(fields) {
            assert_eq!(*kind, pinned_kind);
            assert_eq!(*list, pinned, "type {kind}");
            for &f in *list {
                assert_eq!(hex(tested_default(f)), expected(f), "type {kind} field {f}");
            }
        }
    }

    /// The command settings and their tested values, as literal text. The
    /// test writer takes its values from `COMMAND_SETTINGS`. This test does
    /// not, so a wrong value cannot hide in both.
    #[test]
    fn test_command_settings_are_pinned() {
        let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
        let pinned: Vec<(usize, &str)> = vec![
            (5, "ffffffff"),
            (8, "00"),
            (10, "00000000"),
            (11, "00000000"),
            (12, "00000000"),
            (13, "00000000"),
            (16, "00"),
            (17, "02000000"),
            (18, "00000000"),
            (23, "00000000"),
            (24, "00"),
            (25, "00000000"),
            (26, "00000000"),
            (27, "00000000"),
            (28, "00000000"),
            (29, "00"),
            (30, "00"),
            (31, "00"),
            (34, "00"),
            (35, "00"),
            (36, "01"),
            (38, "00"),
            (39, "00"),
            (40, "00"),
            (41, "00"),
            (42, "00"),
            (43, "00"),
            (44, "00"),
            (45, "00"),
            (46, "00"),
            (47, "00"),
            (48, "0101"),
            (49, "00"),
            (51, "00"),
            (52, "ffffffff"),
            (53, "00"),
            (54, "00"),
            (55, "00"),
            (56, "00"),
            (57, "00000000"),
            (58, "01000000"),
            (59, "00"),
            (60, "00"),
            (61, "ffffffff"),
            (62, "ffffffff"),
            (63, "00000000"),
            (64, "00"),
            (65, "00"),
            (66, "0000000000000000000000000000000000"),
            (68, "00000000"),
            (69, "00000000"),
            (70, "00000000"),
            (71, "00000000"),
            (72, "00000000"),
            (73, "00000000"),
            (75, "00000000"),
            (77, "00"),
            (78, "00"),
            (79, "00"),
            (80, "ffffffff"),
            (81, "00"),
            (82, "ffffffff"),
        ];
        let actual: Vec<(usize, String)> =
            COMMAND_SETTINGS.iter().map(|(k, v)| (*k, hex(v))).collect();
        let pinned: Vec<(usize, String)> = pinned
            .into_iter()
            .map(|(k, v)| (k, v.to_string()))
            .collect();
        assert_eq!(actual, pinned);
        assert_eq!(COMMAND_STRING_FIELDS, [5, 52, 61, 62, 80, 82]);
        // Not constant in the tested profile: not in the list.
        for k in [9, 22, 32, 37, 74, 76] {
            assert!(COMMAND_SETTINGS.iter().all(|(f, _)| *f != k), "{k}");
        }
    }

    #[test]
    fn test_execute_mode_and_code_hash_are_read() {
        let acts = |list: Vec<Vec<u8>>| {
            walk(&profile(vec![command("x", list, 0, None)]))
                .unwrap()
                .commands[0]
                .actions
                .clone()
        };
        let a = acts(vec![
            exec_by_name("y"),
            exec_with([1; 16], None, false),
            set_field(exec_by_name("y"), 3, 2f64.to_le_bytes().to_vec()),
        ]);
        assert_eq!(a[0].exec_by_name, Some(true));
        assert_eq!(a[1].exec_by_name, Some(false));
        assert_eq!(a[2].exec_by_name, None);
        let code = inline_code("x");
        let base = acts(vec![code.clone()])[0].code_hash.unwrap();
        for k in [5, 6, 11] {
            let mut f = fields_of(&code);
            let mut changed = f[k].clone();
            let last = changed.len() - 1;
            changed[last] ^= 1;
            f[k] = changed;
            let other = acts(vec![record(f)])[0].code_hash.unwrap();
            assert_ne!(other, base, "field {k}");
        }
    }

    #[test]
    fn test_command_enabled_and_voice_trigger_are_read() {
        let c = |k: usize, v: Vec<u8>| {
            let mut f = command_fields("x", vec![], 0, None);
            f[k] = v;
            walk(&profile_raw(vec![record(f)])).unwrap().commands[0].clone()
        };
        let plain = c(3, vec![1]);
        assert!(plain.enabled && plain.voice_trigger);
        assert!(!c(4, vec![0]).enabled);
        assert!(!c(15, vec![1, 0]).voice_trigger);
    }

    #[test]
    fn test_limits_give_typed_errors() {
        let small = Limits {
            fields: 1024,
            commands: 2,
            actions: 3,
            actions_per_command: 2,
        };
        let cmds = |n: usize, acts: usize| -> Vec<Vec<u8>> {
            (0..n)
                .map(|_| command("x", (0..acts).map(|_| sound()).collect(), 0, None))
                .collect()
        };
        assert_eq!(
            walk_with(&profile(cmds(3, 0)), small),
            Err(BuildError::TooMany {
                what: "commands",
                limit: 2
            })
        );
        assert_eq!(
            walk_with(&profile(cmds(1, 3)), small),
            Err(BuildError::TooMany {
                what: "actions in one command",
                limit: 2
            })
        );
        assert_eq!(
            walk_with(&profile(cmds(2, 2)), small),
            Err(BuildError::TooMany {
                what: "actions",
                limit: 3
            })
        );
        // A header count above the limit, with a table that fits.
        let mut root = fields_of(&profile(vec![]));
        root.resize(1026, null_str());
        assert!(matches!(
            walk(&record(root)),
            Err(BuildError::TooManyFields {
                kind: RecordKind::Profile,
                maximum: 1024,
                found: 1025,
                ..
            })
        ));
        for e in [
            BuildError::TooMany {
                what: "commands",
                limit: 2,
            },
            BuildError::Version,
            BuildError::OlderFormat,
        ] {
            assert!(!e.to_string().contains('{'), "{e}");
        }
    }

    #[test]
    fn test_inflate_round_trip() {
        let data = profile(vec![command("hello", vec![write("Hello")], 0, None)]);
        assert_eq!(inflate(&deflate(&data)).unwrap(), data);
    }

    #[test]
    fn test_inflate_rejects_trailing_bytes() {
        let mut packed = deflate(&profile(vec![]));
        packed.extend_from_slice(b"junk");
        assert!(matches!(
            inflate(&packed),
            Err(BuildError::TrailingBytes { .. })
        ));
    }

    #[test]
    fn test_inflate_rejects_a_truncated_stream() {
        let packed = deflate(&profile(vec![command("a", vec![write("A")], 0, None)]));
        assert!(matches!(
            inflate(&packed[..packed.len() / 2]),
            Err(BuildError::Truncated | BuildError::Inflate { .. })
        ));
    }

    #[test]
    fn test_inflate_rejects_garbage() {
        assert!(matches!(
            inflate(&[0xff; 32]),
            Err(BuildError::Inflate { .. })
        ));
    }

    #[test]
    fn test_root_size_must_be_the_length() {
        let mut data = profile(vec![]);
        data.push(0);
        assert!(matches!(walk(&data), Err(BuildError::RootSize { .. })));
        assert!(matches!(walk(&[1, 2]), Err(BuildError::RootSize { .. })));
    }

    #[test]
    fn test_non_monotonic_offsets_fail() {
        let mut data = profile(vec![]);
        // Offset entry 3 of the root record: make it smaller than entry 2.
        let at = 8 + 4 * 3;
        data[at..at + 4].copy_from_slice(&0i32.to_le_bytes());
        assert!(matches!(
            walk(&data),
            Err(BuildError::BadOffsets { offset: 0 })
        ));
    }

    #[test]
    fn test_offset_past_record_end_fails() {
        let mut act = action_fields(14);
        act.truncate(3);
        let mut rec = record(act);
        // The last offset points past the record end.
        let size = rec.len() as i32;
        let at = 8 + 4 * 2;
        rec[at..at + 4].copy_from_slice(&(size + 1).to_le_bytes());
        assert!(matches!(
            Reader {
                data: &rec,
                limits: LIMITS
            }
            .record(0, rec.len(), RecordKind::Action, 0),
            Err(BuildError::BadOffsets { .. })
        ));
    }

    #[test]
    fn test_field_counts_below_the_minimum_fail() {
        let mut cmd = command_fields("x", vec![], 0, None);
        cmd.truncate(MIN_FIELDS.command as usize);
        let data = profile_raw(vec![record(cmd)]);
        assert!(matches!(
            walk(&data),
            Err(BuildError::FieldCount {
                kind: RecordKind::Command,
                minimum: 20,
                found: 19,
                ..
            })
        ));
    }

    #[test]
    fn test_larger_and_smaller_field_counts_are_read() {
        let mut act = action_fields(14);
        act.push(vec![0]);
        let mut small = action_fields(21);
        small.truncate(MIN_FIELDS.action as usize + 1);
        let data = profile(vec![command(
            "x",
            vec![record(act), record(small)],
            0,
            None,
        )]);
        let p = walk(&data).unwrap();
        assert_eq!(p.counts.action, [30, 33].into());
    }

    #[test]
    fn test_unknown_command_type_is_read() {
        let data = profile(vec![command("x", vec![], 3, None)]);
        assert_eq!(walk(&data).unwrap().commands[0].command_type, 3);
    }

    #[test]
    fn test_unknown_operator_is_read() {
        let data = profile(vec![command(
            "x",
            vec![condition("v", "a", 7), end_condition()],
            0,
            None,
        )]);
        assert_eq!(walk(&data).unwrap().commands[0].actions[0].operator, 7);
    }

    #[test]
    fn test_list_that_does_not_fill_its_field_fails() {
        let mut cmd = command_fields("x", vec![], 0, None);
        // Count 1, but no record follows.
        cmd[2] = 1i32.to_le_bytes().to_vec();
        let data = profile_raw(vec![record(cmd)]);
        assert!(matches!(walk(&data), Err(BuildError::BadList { .. })));
    }

    #[test]
    fn test_list_count_larger_than_its_field_fails_before_allocation() {
        let mut cmd = command_fields("x", vec![], 0, None);
        cmd[2] = 100i32.to_le_bytes().to_vec();
        let data = profile_raw(vec![record(cmd)]);
        assert!(matches!(walk(&data), Err(BuildError::BadList { .. })));
        // A count above the limit stops before the size check.
        let mut cmd = command_fields("x", vec![], 0, None);
        cmd[2] = i32::MAX.to_le_bytes().to_vec();
        let data = profile_raw(vec![record(cmd)]);
        assert!(matches!(walk(&data), Err(BuildError::TooMany { .. })));
    }

    #[test]
    fn test_bad_string_length_fails() {
        let mut cmd = command_fields("x", vec![], 0, None);
        cmd[1] = [5i32.to_le_bytes().to_vec(), b"abc".to_vec()].concat();
        let data = profile_raw(vec![record(cmd)]);
        assert!(matches!(
            walk(&data),
            Err(BuildError::BadField {
                field: 1,
                expected: "a string",
                ..
            })
        ));
    }

    #[test]
    fn test_record_outside_parent_fails() {
        let mut data = profile(vec![command("x", vec![], 0, None)]);
        // The size of the first command record (after the root header, the
        // root offset table and the fields 0 and 1, and the list count).
        let root_fields = TESTED_FIELD_COUNTS.profile as usize + 1;
        let list_at = 8 + 4 * root_fields + 16 + 8;
        let at = list_at + 4;
        let size = i32::from_le_bytes(data[at..at + 4].try_into().unwrap());
        data[at..at + 4].copy_from_slice(&(size + 100).to_le_bytes());
        assert!(walk(&data).is_err());
    }
}
