use std::fmt;

/// The kind of a record in the profile tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordKind {
    Profile,
    Command,
    Action,
}

impl fmt::Display for RecordKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            RecordKind::Profile => "profile",
            RecordKind::Command => "command",
            RecordKind::Action => "action",
        })
    }
}

/// Why the builder cannot make a map from the profile bytes.
///
/// Each error is about the structure of the whole file. The builder never
/// guesses: a file that it cannot read gives an error, not a partial map.
/// A single command that the builder does not understand is not an error:
/// the builder leaves that command out (see `LeftOutClass::NotUnderstood`).
/// `offset` is a byte position in the inflated profile. The texts are for
/// a person: fixed words and numbers only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildError {
    /// The bytes are not one raw DEFLATE stream. `offset`: the input byte
    /// where the inflate step stopped.
    Inflate { offset: usize },
    /// The DEFLATE stream stops before its end (a cut or empty file).
    Truncated,
    /// The DEFLATE stream ends before the end of the file.
    TrailingBytes { used: usize, total: usize },
    /// The inflated profile is larger than the limit.
    TooLarge { limit: usize },
    /// The size of the root record is not the inflated length.
    RootSize { declared: i64, actual: usize },
    /// A record header or field is outside its parent.
    OutOfBounds { offset: usize, what: &'static str },
    /// The offset table of a record is not monotonic, or it points
    /// outside the record.
    BadOffsets { offset: usize },
    /// A record has fewer fields than the rules read.
    FieldCount {
        kind: RecordKind,
        offset: usize,
        minimum: u32,
        found: i64,
    },
    /// A record has more fields than the walk accepts.
    TooManyFields {
        kind: RecordKind,
        offset: usize,
        maximum: u32,
        found: i64,
    },
    /// The profile has more items of one kind than the walk accepts.
    TooMany { what: &'static str, limit: usize },
    /// A field of the length signature has another length: a field moved
    /// or has another encoding.
    Layout {
        kind: RecordKind,
        offset: usize,
        field: usize,
        length: usize,
    },
    /// The VoiceAttack version text (profile field 32) does not start
    /// with digits, a dot, and digits.
    Version,
    /// Many records have another value in a field that has one value in
    /// each record of its kind in the tested profile: an action field of
    /// `record::SHARED_FIELDS`, or a command field of
    /// `record::COMMAND_SETTINGS`. The values can differ. The program that
    /// wrote the profile changed its format: this is not a setting of the
    /// author. `records`: the records with another value, of `total`.
    ///
    /// `action_type`: the action type when the count is of the actions of
    /// one type (`total` is then the number of actions of that type).
    FormatChanged {
        kind: RecordKind,
        action_type: Option<i32>,
        field: usize,
        records: usize,
        total: usize,
    },
    /// One reason that points at a change of the format (an unknown
    /// setting of an action or of a command, or an unknown action type
    /// before the write) leaves out more than a quarter of the commands
    /// with a request write. `total`: the commands with a request write.
    ReasonShare {
        reason: String,
        commands: usize,
        total: usize,
    },
    /// Some records have fewer fields than the tested format. The meaning
    /// of the fields in an older format is not checked.
    OlderFormat,
    /// A field that a rule reads does not have the expected encoding.
    BadField {
        offset: usize,
        field: usize,
        expected: &'static str,
    },
    /// A list does not fill its field exactly.
    BadList { offset: usize },
    /// The counts of the result show that the file is not a usable
    /// profile.
    Totals(Totals),
}

/// A check of the counts at the end of the build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Totals {
    /// The writes in the records and in the left-out commands do not add
    /// up to the writes in the profile.
    Writes {
        in_records: usize,
        left_out: usize,
        in_profile: usize,
    },
    /// The map has fewer records than a real profile has.
    /// `largest_reason`: the reason of the largest group of left-out
    /// commands, with the number of commands.
    TooFewRecords {
        found: usize,
        minimum: usize,
        largest_reason: Option<(String, usize)>,
    },
    /// No record writes a fixed token.
    NoFixedRecord,
    /// No command of the profile writes a request.
    NoRequestWrite,
}

/// The action for a profile format that the program does not know.
const DEVELOPER: &str = "tell the developer";
/// The action for a file that is not a VoiceAttack profile.
const OTHER_FILE: &str = "use the profile file of EDCoPilot";
/// The action for a profile file that is damaged or that EDCoPilot writes
/// at this moment.
const WRITE_AGAIN: &str = "try again, or start EDCoPilot one time to make the file again";

impl fmt::Display for Totals {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Totals::Writes {
                in_records,
                left_out,
                in_profile,
            } => write!(
                f,
                "the program did not count each request write of the profile: the profile has \
                 {in_profile} request writes, the map has {in_records} and the left-out commands \
                 have {left_out}, {DEVELOPER}"
            ),
            Totals::TooFewRecords {
                found,
                minimum,
                largest_reason,
            } => {
                write!(
                    f,
                    "the map has {found} commands, a profile of EDCoPilot has at least {minimum}"
                )?;
                if let Some((reason, n)) = largest_reason {
                    write!(
                        f,
                        ", the largest group of left-out commands ({n}) has the reason \"{reason}\""
                    )?;
                }
                write!(f, ", {DEVELOPER}")
            }
            Totals::NoFixedRecord => write!(
                f,
                "no command of the map writes a fixed request, {DEVELOPER}"
            ),
            Totals::NoRequestWrite => write!(
                f,
                "no command of the profile writes a request to EDCoPilot, {DEVELOPER}"
            ),
        }
    }
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BuildError::Inflate { offset } => write!(
                f,
                "the file is not a VoiceAttack profile: the data at byte {offset} is not \
                 compressed profile data, {OTHER_FILE}"
            ),
            BuildError::Truncated => write!(
                f,
                "the profile file is empty or cut off: the compressed data stops too early, \
                 {WRITE_AGAIN}"
            ),
            BuildError::TrailingBytes { used, total } => write!(
                f,
                "the compressed profile ends at byte {used} of {total}: unknown bytes follow, \
                 {WRITE_AGAIN}"
            ),
            BuildError::TooLarge { limit } => {
                write!(
                    f,
                    "the inflated profile is larger than {limit} bytes, {DEVELOPER}"
                )
            }
            BuildError::RootSize { declared, actual } => write!(
                f,
                "the file is not a VoiceAttack profile: the root record size is {declared}, \
                 the inflated data has {actual} bytes, {OTHER_FILE}"
            ),
            BuildError::OutOfBounds { offset, what } => {
                write!(
                    f,
                    "{what} at byte {offset} is outside its parent, {DEVELOPER}"
                )
            }
            BuildError::BadOffsets { offset } => {
                write!(
                    f,
                    "the offset table of the record at byte {offset} is not valid, {DEVELOPER}"
                )
            }
            BuildError::FieldCount {
                kind,
                offset,
                minimum,
                found,
            } => write!(
                f,
                "the {kind} record at byte {offset} has the field count {found}, the program \
                 needs at least {minimum}, {DEVELOPER}"
            ),
            BuildError::TooManyFields {
                kind,
                offset,
                maximum,
                found,
            } => write!(
                f,
                "the {kind} record at byte {offset} has the field count {found}, the program \
                 accepts at most {maximum}, {DEVELOPER}"
            ),
            BuildError::TooMany { what, limit } => {
                write!(f, "the profile has more than {limit} {what}, {DEVELOPER}")
            }
            BuildError::Layout {
                kind,
                offset,
                field,
                length,
            } => write!(
                f,
                "the format of the profile is not understood: field {field} of the {kind} record \
                 at byte {offset} does not have the tested length of {length} bytes, {DEVELOPER}"
            ),
            BuildError::Version => write!(
                f,
                "the format of the profile is not understood: the VoiceAttack version text is \
                 not a version number, {DEVELOPER}"
            ),
            BuildError::FormatChanged {
                kind,
                action_type,
                field,
                records,
                total,
            } => {
                let of_type = match action_type {
                    Some(super::record::OTHER_TYPES) => " of other types".to_string(),
                    Some(t) => format!(" of type {t}"),
                    None => String::new(),
                };
                write!(
                    f,
                    "the format of the profile is not understood: {records} of {total} \
                     {kind}s{of_type} have another value in {kind} field {field}, the profile \
                     has a newer format than the tested format, {DEVELOPER}"
                )
            }
            BuildError::ReasonShare {
                reason,
                commands,
                total,
            } => write!(
                f,
                "the format of the profile is not understood: {commands} of {total} commands \
                 with a request write are left out with the reason \"{reason}\", the profile \
                 has a newer format than the tested format, {DEVELOPER}"
            ),
            BuildError::OlderFormat => f.write_str(
                "the profile has an older format than the program knows, update EDCoPilot",
            ),
            BuildError::BadField {
                offset,
                field,
                expected,
            } => write!(
                f,
                "field {field} of the record at byte {offset} is not {expected}, {DEVELOPER}"
            ),
            BuildError::BadList { offset } => {
                write!(
                    f,
                    "the list at byte {offset} does not fill its field, {DEVELOPER}"
                )
            }
            BuildError::Totals(t) => write!(f, "the profile is not usable: {t}"),
        }
    }
}

impl std::error::Error for BuildError {}

#[cfg(test)]
mod tests {
    use super::*;

    /// The last words of each text: the action of the user.
    const ACTIONS: [&str; 4] = [
        ", tell the developer",
        ", update EDCoPilot",
        ", use the profile file of EDCoPilot",
        ", try again, or start EDCoPilot one time to make the file again",
    ];

    fn every_error() -> Vec<BuildError> {
        let kind = RecordKind::Command;
        vec![
            BuildError::Inflate { offset: 1 },
            BuildError::Truncated,
            BuildError::TrailingBytes { used: 1, total: 2 },
            BuildError::TooLarge { limit: 1 },
            BuildError::RootSize {
                declared: 1,
                actual: 2,
            },
            BuildError::OutOfBounds {
                offset: 1,
                what: "a field",
            },
            BuildError::BadOffsets { offset: 1 },
            BuildError::FieldCount {
                kind,
                offset: 1,
                minimum: 2,
                found: 1,
            },
            BuildError::TooManyFields {
                kind,
                offset: 1,
                maximum: 2,
                found: 3,
            },
            BuildError::TooMany {
                what: "commands",
                limit: 1,
            },
            BuildError::Layout {
                kind,
                offset: 1,
                field: 2,
                length: 3,
            },
            BuildError::Version,
            BuildError::FormatChanged {
                kind,
                action_type: Some(4),
                field: 2,
                records: 3,
                total: 9,
            },
            BuildError::ReasonShare {
                reason: "r".into(),
                commands: 3,
                total: 9,
            },
            BuildError::OlderFormat,
            BuildError::BadField {
                offset: 1,
                field: 2,
                expected: "a text",
            },
            BuildError::BadList { offset: 1 },
            BuildError::Totals(Totals::Writes {
                in_records: 1,
                left_out: 1,
                in_profile: 3,
            }),
            BuildError::Totals(Totals::TooFewRecords {
                found: 1,
                minimum: 2,
                largest_reason: Some(("r".into(), 1)),
            }),
            BuildError::Totals(Totals::TooFewRecords {
                found: 1,
                minimum: 2,
                largest_reason: None,
            }),
            BuildError::Totals(Totals::NoFixedRecord),
            BuildError::Totals(Totals::NoRequestWrite),
        ]
    }

    #[test]
    fn test_each_error_text_ends_with_an_action() {
        for error in every_error() {
            let text = error.to_string();
            assert!(
                ACTIONS.iter().any(|action| text.ends_with(action)),
                "{text}"
            );
            assert_eq!(
                ACTIONS
                    .iter()
                    .filter(|action| text.contains(*action))
                    .count(),
                1,
                "{text}"
            );
        }
    }
}
