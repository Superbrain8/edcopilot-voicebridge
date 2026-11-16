//! The extraction rules of the first map extractor (a Python script), on
//! the walked profile, and the whitelist of `allow.rs`. The classes of the
//! script keep their order and their meaning. A command that the script
//! puts into the map goes into the map only when the whitelist allows it.
//! See `docs/PROFILE_FORMAT.md`.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, HashSet};

use super::allow::{self, Index};
use super::error::{BuildError, Totals};
use super::record::{
    RawAction, RawCommand, RawProfile, COND_BEGIN, COND_END, INLINE_CODE, KEY_PRESS, LOOP_BEGIN,
    LOOP_END, PLUGIN, SET_TEXT, SET_TEXT_FROM_INT, WAIT_FOR_SPEECH, WRITE_FILE,
};
use super::report::{BlockingPhrase, LeftOut, LeftOutClass};
use crate::command_map::{Command, CommandClass, CommandType};
use crate::dynamic_vocab::slot_kind;
use crate::grammar::{parse_grammar, GrammarNode};
use crate::number_phrase::{number_text_outside_ranges, NUMBER_TEXT_REASON};

/// The path of the request-file write.
pub const REQUEST_PATH: &str = r"{TXT:PythonLibPath}\{TXT:RequestFile}";
pub(super) const NUM_OF_CMD: &str = "{TXTNUM:LastCMD}";
const SWITCHED_OFF_COMMAND: &str =
    "the command is switched off or has no voice trigger in the profile";
const SWITCHED_OFF_GROUP: &str =
    "a prefix of the composite group has an action that is switched off in the profile";

/// The longest phrase (in characters) that the builder reads. The tested
/// profile has phrases of at most 434 characters. A longer phrase leaves
/// its command out. Its first characters give the blocking phrase, only
/// when each group and each variable in them closes (see `blocking_of`).
pub(super) const MAX_PHRASE_CHARS: usize = 4_096;

/// The longest request text (in characters) that the builder reads. The
/// tested profile has tokens of at most 205 characters.
pub(super) const MAX_TOKEN_CHARS: usize = 1_024;

/// A string field as the script reads it: an empty string is `None`.
pub(super) fn text(field: &Option<String>) -> Option<&str> {
    field.as_deref().filter(|s| !s.is_empty())
}

pub(super) fn is_write(a: &RawAction) -> bool {
    a.kind == WRITE_FILE && text(&a.text6) == Some(REQUEST_PATH)
}

/// The request-file writes of a command, in action order: the index of
/// the action and the token.
fn writes(c: &RawCommand) -> Vec<(usize, Option<&str>)> {
    c.actions
        .iter()
        .enumerate()
        .filter(|(_, a)| is_write(a))
        .map(|(k, a)| (k, text(&a.text5)))
        .collect()
}

/// The (name, value) of the set-text actions that have a name.
fn sets_of(c: &RawCommand) -> Vec<(&str, Option<&str>)> {
    c.actions
        .iter()
        .filter(|a| a.kind == SET_TEXT)
        .filter_map(|a| text(&a.text5).map(|n| (n, text(&a.text6))))
        .collect()
}

/// The block depth of the first request-file write.
fn write_depth(c: &RawCommand) -> Option<i32> {
    let mut depth = 0;
    for a in &c.actions {
        match a.kind {
            COND_BEGIN | LOOP_BEGIN => depth += 1,
            COND_END | LOOP_END => depth -= 1,
            _ if is_write(a) => return Some(depth),
            _ => {}
        }
    }
    None
}

/// A placeholder of a token or a phrase.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Place<'a> {
    /// `{TXTNUM:name}`.
    Num(&'a str),
    /// `{TXT:name}`.
    Txt(&'a str),
    Clip,
    Cmd,
}

/// The placeholders of `text`, left to right, as the regular expression
/// `\{TXTNUM:[^}]+\}|\{TXT:[^}]+\}|\{CLIP\}|\{CMD\}` finds them. The work
/// is linear in the length of the text: each `{` before a `}` has that `}`
/// as the end of its placeholder, so the search tries these `{` in order
/// and then goes on after the `}`.
pub(super) fn placeholders(text: &str) -> Vec<Place<'_>> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(rel) = text[i..].find('{') {
        let open = i + rel;
        let Some(close) = text[open..].find('}').map(|k| open + k) else {
            break;
        };
        let mut at = open;
        loop {
            if let Some(p) = place(&text[at + 1..close]) {
                out.push(p);
                break;
            }
            // No match at this `{`: try the next `{` before the `}`.
            match text[at + 1..close].find('{') {
                Some(k) => at += 1 + k,
                None => break,
            }
        }
        i = close + 1;
    }
    out
}

/// The placeholder with the text `inner` between `{` and `}`.
fn place(inner: &str) -> Option<Place<'_>> {
    if let Some(name) = inner.strip_prefix("TXTNUM:") {
        (!name.is_empty()).then_some(Place::Num(name))
    } else if let Some(name) = inner.strip_prefix("TXT:") {
        (!name.is_empty()).then_some(Place::Txt(name))
    } else if inner == "CLIP" {
        Some(Place::Clip)
    } else if inner == "CMD" {
        Some(Place::Cmd)
    } else {
        None
    }
}

/// The names of the `{TXT:name}` placeholders.
pub(super) fn txt_vars(text: &str) -> Vec<&str> {
    placeholders(text)
        .into_iter()
        .filter_map(|p| match p {
            Place::Txt(n) => Some(n),
            _ => None,
        })
        .collect()
}

/// `name` matches `prefix` and then one or more ASCII digits.
fn prefix_digits(name: &str, prefix: &str) -> bool {
    name.strip_prefix(prefix)
        .is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
}

fn var_is_name(v: &str) -> bool {
    matches!(v, "SystemName" | "spokenQueuedName" | "LastCMD") || prefix_digits(v, "Group")
}

/// The class and the variable source of a token template.
fn classify(token: &str) -> (CommandClass, Option<String>) {
    let places = placeholders(token);
    if places.is_empty() {
        return (CommandClass::Fixed, None);
    }
    let mut vars = Vec::new();
    let mut name_hit = false;
    for p in places {
        match p {
            Place::Cmd => {
                vars.push("CMD");
                name_hit = true;
            }
            Place::Clip => vars.push("CLIP"),
            Place::Num(v) | Place::Txt(v) => {
                vars.push(v);
                name_hit |= var_is_name(v);
            }
        }
    }
    let class = if name_hit {
        CommandClass::DynamicName
    } else {
        CommandClass::DynamicValue
    };
    (class, Some(vars.join(",")))
}

fn owned(writes: &[(usize, Option<&str>)]) -> Vec<String> {
    writes
        .iter()
        .map(|(_, t)| t.unwrap_or("").to_string())
        .collect()
}

/// The records, the left-out commands and the profile variables.
pub(super) struct Extracted {
    pub commands: Vec<Command>,
    pub left_out: Vec<LeftOut>,
    pub profile_vars: HashMap<String, String>,
    /// The phrases of the left-out commands that VoiceAttack recognises
    /// by voice.
    pub blocking: Vec<BlockingPhrase>,
    /// The commands with one or more request writes.
    pub writers: usize,
    /// See `BuildReport::unknown_phrase_vars`.
    pub unknown_phrase_vars: usize,
    /// See `BuildReport::cannot_fire`.
    pub cannot_fire: usize,
}

/// The reason of `command_setting` and of `allow::not_allowed` for an
/// action type that the table does not list, before the write.
const UNKNOWN_TYPE_BEFORE: &str = " (not known) before the write";

/// True for a reason that points at a change of the format: an unknown
/// setting of an action or of a command, or an unknown action type before
/// the write. Many commands with one such reason stop the build (see
/// `BuildError::ReasonShare`).
pub(super) fn format_reason(reason: &str) -> bool {
    reason == allow::UNKNOWN_SETTING
        || (reason.starts_with(COMMAND_SETTING_START) && reason.ends_with(COMMAND_SETTING_END))
        || (reason.starts_with("action type ") && reason.ends_with(UNKNOWN_TYPE_BEFORE))
}

/// The blocking phrase of the left-out command `c`, or `None` when
/// VoiceAttack does not recognise the command by voice:
///
/// - the command is switched off or has no voice trigger (fields 4 and 15).
///   A field that the walk cannot read counts as on;
/// - the phrase is empty, or only spaces;
/// - the phrase is longer than [`MAX_PHRASE_CHARS`], and a group or a
///   variable is open at the cut;
/// - the phrase is an internal command: it starts with `((` and ends with
///   `))` (the form of the commands that other commands execute);
/// - the command type is not known (not 0, 1 or 2);
/// - a prefix or a suffix has no composite group, or a suffix has no
///   prefix in its group (class `orphan_suffix`). VoiceAttack recognises a
///   suffix only after a prefix of its group.
///
/// Each class can hold a spoken command: `no_write` (the command does
/// something else, for example a key press or a speech), `multi_write`,
/// `maintenance`, and `not_understood`.
fn blocking_of(c: &RawCommand, class: LeftOutClass) -> Option<BlockingPhrase> {
    if !c.enabled || !c.voice_trigger || class == LeftOutClass::Orphan {
        return None;
    }
    let phrase = text(&c.phrase)?.trim();
    if phrase.is_empty() || (phrase.starts_with("((") && phrase.ends_with("))")) {
        return None;
    }
    let cut: String;
    let phrase = if too_long(phrase, MAX_PHRASE_CHARS) {
        // Fix round 10: a longer phrase blocks with its first characters,
        // when the cut does not split a group or a variable.
        cut = phrase.chars().take(MAX_PHRASE_CHARS).collect();
        let first = cut.trim();
        if first.is_empty() || !whole_groups(first) {
            return None;
        }
        first
    } else {
        phrase
    };
    let command_type = match c.command_type {
        0 => CommandType::Full,
        1 => CommandType::Prefix,
        2 => CommandType::Suffix,
        _ => return None,
    };
    let composite_group = text(&c.group).map(str::to_string);
    if command_type != CommandType::Full && composite_group.is_none() {
        return None;
    }
    Some(BlockingPhrase {
        phrase: phrase.to_string(),
        class,
        command_type,
        composite_group: (command_type != CommandType::Full)
            .then_some(composite_group)
            .flatten(),
    })
}

/// The decision for one command: a record, or left out.
enum Decision {
    Keep(Box<Command>),
    Leave(LeftOutClass, String),
}

use Decision::{Keep, Leave};

const COMMAND_SETTING_START: &str = "the command has a setting in field ";
const COMMAND_SETTING_END: &str = " that the program does not know";

/// The reason of a command with an unknown setting in `field`.
fn command_setting(field: usize) -> String {
    format!("{COMMAND_SETTING_START}{field}{COMMAND_SETTING_END}")
}

fn not_understood(reason: impl Into<String>) -> Decision {
    Leave(LeftOutClass::NotUnderstood, reason.into())
}

/// The context of the whole profile that the decision for one command
/// needs.
struct Context<'a> {
    prefix_groups: HashSet<&'a str>,
    /// The composite groups with a prefix that has an action that is
    /// switched off (field 28 = 1).
    switched_off_groups: HashSet<&'a str>,
    categories: HashSet<&'a str>,
    index: Index<'a>,
    /// The writers of the variables of the profile.
    writers: Writers<'a>,
    /// The name patterns of the sets with a token in the variable name.
    patterns: NamePatterns,
    /// A variable of the request path has a writer that the builder does
    /// not model (see [`request_path_writer`]).
    path_writer: bool,
}

pub(super) fn extract(profile: &RawProfile, reviewed: &[&str]) -> Result<Extracted, BuildError> {
    let cmds = &profile.commands;
    let prefixes = || cmds.iter().filter(|c| c.command_type == 1);
    let writers = Writers::new(cmds);
    let patterns = NamePatterns::new(&writers);
    let ctx = Context {
        prefix_groups: prefixes().filter_map(|c| text(&c.group)).collect(),
        switched_off_groups: prefixes()
            .filter(|c| c.actions.iter().any(|a| a.flag28))
            .filter_map(|c| text(&c.group))
            .collect(),
        categories: cmds.iter().filter_map(|c| text(&c.category)).collect(),
        index: Index::new(cmds, reviewed),
        path_writer: request_path_writer(&writers, &patterns),
        writers,
        patterns,
    };
    let mut records = Vec::new();
    let mut left_out = Vec::new();
    let mut blocking = Vec::new();
    let mut writers = 0;
    for c in cmds {
        let w = writes(c);
        writers += usize::from(!w.is_empty());
        match decide(c, &w, &ctx) {
            Keep(rec) => records.push(*rec),
            Leave(class, reason) => {
                blocking.extend(blocking_of(c, class));
                left_out.push(LeftOut {
                    phrase: text(&c.phrase).unwrap_or("").to_string(),
                    class,
                    tokens: owned(&w),
                    reason,
                })
            }
        }
    }
    validate(cmds, &records, &left_out)?;
    let profile_vars = profile_vars(&ctx, &records);
    let unknown_phrase_vars = records
        .iter()
        .filter(|r| {
            txt_vars(r.phrase.as_deref().unwrap_or(""))
                .into_iter()
                .any(|v| {
                    !v.starts_with("EDCP_")
                        && slot_kind(v).is_none()
                        && !profile_vars.contains_key(v)
                })
        })
        .count();
    let cannot_fire = records.iter().filter(|r| !can_fire(r)).count();
    Ok(Extracted {
        commands: records,
        left_out,
        profile_vars,
        blocking,
        writers,
        unknown_phrase_vars,
        cannot_fire,
    })
}

/// The placeholders that `fallback` names take the only captured slot in
/// the matcher.
const FALLBACK_VARS: [&str; 2] = ["SystemName", "spokenQueuedName"];

/// False when a placeholder of the token of `rec` has no fill path in the
/// matcher: an unclosed `{`, another VoiceAttack token (each placeholder
/// other than `{CMD}` and `{TXT:name}`), or a text variable that is no
/// constant, no number variable, no phrase slot and no fallback of a
/// phrase with a slot. A suffix takes its text variables from the prefix,
/// so they count as filled. A prefix has no token. The check is generous:
/// true does not mean that the record works.
fn can_fire(rec: &Command) -> bool {
    if rec.command_type == CommandType::Prefix {
        return true;
    }
    let phrase = rec.phrase.as_deref().unwrap_or("");
    let phrase_vars = txt_vars(phrase);
    let mut rest = rec.token_template.as_str();
    while let Some(open) = rest.find('{') {
        let Some(close) = rest[open..].find('}').map(|k| open + k) else {
            return false;
        };
        let inner = &rest[open + 1..close];
        let filled = match inner.strip_prefix("TXT:") {
            _ if inner == "CMD" => true,
            Some(v) => {
                rec.command_type == CommandType::Suffix
                    || rec.constants.contains_key(v)
                    || rec.number_vars.iter().any(|n| n == v)
                    || phrase_vars.contains(&v)
                    || (FALLBACK_VARS.contains(&v) && !phrase_vars.is_empty())
            }
            None => false,
        };
        if !filled {
            return false;
        }
        rest = &rest[close + 1..];
    }
    true
}

/// The decision for one command. First the classes of the script, in its
/// order. Then the whitelist for each command that the script keeps.
fn decide(c: &RawCommand, w: &[(usize, Option<&str>)], ctx: &Context) -> Decision {
    let Some(phrase) = text(&c.phrase) else {
        return not_understood("the command has no phrase");
    };
    if too_long(phrase, MAX_PHRASE_CHARS) {
        return not_understood(format!(
            "the phrase is longer than {MAX_PHRASE_CHARS} characters"
        ));
    }
    let group = text(&c.group);
    let ctype = match c.command_type {
        0 => CommandType::Full,
        1 => CommandType::Prefix,
        2 => CommandType::Suffix,
        other => return not_understood(format!("the command type {other} is not known")),
    };
    if ctype != CommandType::Full && group.is_none() {
        return not_understood("the prefix or suffix has no composite group");
    }
    let category = text(&c.category).map(str::to_string);
    if ctype == CommandType::Suffix && !ctx.prefix_groups.contains(group.unwrap_or("")) {
        return Leave(
            LeftOutClass::Orphan,
            "the composite group has no prefix".into(),
        );
    }
    let category_phrase = ctx.categories.contains(phrase);
    let switched_off_group = ctype != CommandType::Full
        && ctx.switched_off_groups.contains(group.unwrap_or(""))
        && !(ctype == CommandType::Prefix && c.actions.iter().any(|a| a.flag28));
    if ctype == CommandType::Prefix {
        if !w.is_empty() {
            return not_understood("the prefix writes a request token");
        }
        let mut sets = HashMap::new();
        for (n, v) in sets_of(c) {
            let Some(v) = v else {
                return not_understood("the prefix sets a variable to nothing");
            };
            sets.insert(n.to_string(), v.to_string());
        }
        if sets.is_empty() {
            return not_understood("the prefix has no set-text action");
        }
        if category_phrase {
            return not_understood("the phrase is also the name of a category");
        }
        if !c.enabled || !c.voice_trigger {
            return not_understood(SWITCHED_OFF_COMMAND);
        }
        if let Some(field) = c.unknown_setting {
            return not_understood(command_setting(field));
        }
        if let Err(reason) = allow::check_prefix(c) {
            return not_understood(reason);
        }
        if switched_off_group {
            return not_understood(SWITCHED_OFF_GROUP);
        }
        return Keep(Box::new(Command {
            phrase: Some(phrase.to_string()),
            token_template: String::new(),
            class: CommandClass::Fixed,
            category,
            command_type: CommandType::Prefix,
            composite_group: group.map(str::to_string),
            sets,
            ..Command::default()
        }));
    }
    if w.is_empty() {
        return Leave(
            LeftOutClass::NoWrite,
            "the command writes no request token".into(),
        );
    }
    if w.len() > 1 {
        return Leave(
            LeftOutClass::MultiWrite,
            format!("the command writes {} request tokens", w.len()),
        );
    }
    let (wk, token) = (w[0].0, w[0].1.unwrap_or(""));
    if ctype == CommandType::Full {
        if write_depth(c).is_some_and(|d| d != 0) {
            return Leave(
                LeftOutClass::Maintenance,
                "the single write is inside a condition block".into(),
            );
        }
        if c.actions.iter().any(|a| a.kind == KEY_PRESS) {
            return Leave(
                LeftOutClass::Maintenance,
                "the command also presses a key".into(),
            );
        }
    }
    if token.is_empty() {
        return not_understood("the command writes an empty text to the request file");
    }
    if too_long(token, MAX_TOKEN_CHARS) {
        return not_understood(format!(
            "the request text is longer than {MAX_TOKEN_CHARS} characters"
        ));
    }
    if !c.enabled || !c.voice_trigger {
        return not_understood(SWITCHED_OFF_COMMAND);
    }
    if let Some(field) = c.unknown_setting {
        return not_understood(command_setting(field));
    }
    if switched_off_group {
        return not_understood(SWITCHED_OFF_GROUP);
    }
    let mut rec = record_of(c, phrase, token, category);
    let mut region = Vec::new();
    if ctype == CommandType::Suffix {
        let shape = match allow::suffix_shape(&c.actions, token, wk) {
            Ok(shape) => shape,
            Err(reason) => return not_understood(reason),
        };
        rec.command_type = CommandType::Suffix;
        rec.composite_group = group.map(str::to_string);
        rec.prefix_rules = shape.rules;
        region = shape.region;
    }
    if category_phrase {
        return not_understood("the phrase is also the name of a category");
    }
    if ctx.path_writer {
        return not_understood(PATH_WRITER);
    }
    if let Err(reason) = allow::check_record(c, &mut rec, &ctx.index, wk, &region) {
        return not_understood(reason);
    }
    if !rec.number_vars.is_empty() && !number_phrase_with_vars(phrase, ctx) {
        return not_understood(NUMBER_TEXT_REASON);
    }
    Keep(Box::new(rec))
}

/// The record of a full or suffix command with the single write `token`:
/// the class and the number variables of the script. The whitelist
/// (`allow::check_record`) sets the constants.
fn record_of(c: &RawCommand, phrase: &str, token: &str, category: Option<String>) -> Command {
    let (class, var_source) = classify(token);
    let assigns = sets_of(c);
    let token_vars: BTreeSet<&str> = txt_vars(token).into_iter().collect();
    let assigned = |var: &str| -> Vec<Option<&str>> {
        assigns
            .iter()
            .filter(|(n, _)| n.to_lowercase() == var.to_lowercase())
            .map(|(_, v)| *v)
            .collect()
    };
    let mut number_vars = Vec::new();
    let last_cmd = assigned("LastCMD");
    if !last_cmd.is_empty() && last_cmd.iter().all(|v| *v == Some("{CMD}")) {
        number_vars = token_vars
            .iter()
            .filter(|v| assigned(v) == [Some(NUM_OF_CMD)])
            .map(|v| v.to_string())
            .collect();
    }
    Command {
        phrase: Some(phrase.to_string()),
        token_template: token.to_string(),
        class,
        var_source,
        category,
        number_vars,
        ..Command::default()
    }
}

/// The check of the script after the extraction: each request write is in
/// a record or in a left-out command. (The script also checks six live
/// tokens. The library holds no profile content, so it does not.)
fn validate(
    cmds: &[RawCommand],
    records: &[Command],
    left_out: &[LeftOut],
) -> Result<(), BuildError> {
    let total: usize = cmds.iter().map(|c| writes(c).len()).sum();
    let written = records
        .iter()
        .filter(|r| !r.token_template.is_empty())
        .count();
    let left: usize = left_out.iter().map(|l| l.tokens.len()).sum();
    if written + left != total {
        return Err(BuildError::Totals(Totals::Writes {
            in_records: written,
            left_out: left,
            in_profile: total,
        }));
    }
    Ok(())
}

/// The profile variables: a phrase variable of a record with exactly one
/// set-text in the whole profile (names without regard to case), with the
/// literal source and a value without a placeholder. See
/// [`Writers::profile_var`].
fn profile_vars(ctx: &Context, records: &[Command]) -> HashMap<String, String> {
    let phrase_vars: BTreeSet<&str> = records
        .iter()
        .filter_map(|r| r.phrase.as_deref())
        .flat_map(txt_vars)
        .collect();
    phrase_vars
        .into_iter()
        .filter_map(|var| {
            ctx.writers
                .profile_var(&ctx.patterns, var)
                .map(|v| (var.to_string(), v))
        })
        .collect()
}

/// True when each group (`[` to `]`) and each variable (`{` to `}`, `<` to
/// `>`) of `phrase` closes in it, and the parser reads it (the groups are
/// not too deep).
fn whole_groups(phrase: &str) -> bool {
    let mut depth = 0usize;
    let mut open: Option<char> = None;
    for ch in phrase.chars() {
        match (open, ch) {
            (Some(close), _) if ch == close => open = None,
            (Some(_), '{' | '<' | '[' | ']') => return false,
            (Some(_), _) => {}
            (None, '{') => open = Some('}'),
            (None, '<') => open = Some('>'),
            (None, '[') => depth += 1,
            (None, ']') if depth == 0 => return false,
            (None, ']') => depth -= 1,
            (None, _) => {}
        }
    }
    depth == 0 && open.is_none() && parse_grammar(phrase) != GrammarNode::Never
}

/// True when `text` has more than `max` characters.
fn too_long(text: &str, max: usize) -> bool {
    text.len() > max && text.chars().count() > max
}

/// The most distinct variables that [`Writers::profile_var`] checks in one
/// build. A further variable is not a profile variable (a miss). The tested
/// profile has 1 such variable: the other phrase variables are slots or
/// variables of the runtime grammar, which the check skips at once.
const MAX_PROFILE_VAR_LOOKUPS: usize = 64;

/// The most bytes of text of the code and plugin actions that the builder
/// searches for variable names. Above it, each variable counts as named by
/// code. The tested profile has 16,805 bytes.
const MAX_CODE_TEXT: usize = 1024 * 1024;

/// The writers of the variables of the whole profile, by lowercase name.
/// The builder makes this index once, so that each check of a variable
/// does not read all actions again.
struct Writers<'a> {
    /// The set-text actions of each name (field 5).
    set_text: HashMap<String, Vec<&'a RawAction>>,
    /// The types of the set actions of each name (see
    /// `record::SET_NAME_FIELDS`), without the sets with a token in the
    /// name.
    kinds: HashMap<String, Vec<i32>>,
    /// The sets with a token in the variable name.
    token_named: Vec<&'a RawAction>,
    /// The lowercase texts of the code and plugin actions. `None` when they
    /// have more than [`MAX_CODE_TEXT`] bytes: each name then counts as
    /// named by code.
    code: Option<Vec<String>>,
    /// The result of `profile_var` for each variable that it checked.
    memo: RefCell<HashMap<String, Option<String>>>,
}

impl<'a> Writers<'a> {
    fn new(cmds: &'a [RawCommand]) -> Writers<'a> {
        let mut set_text: HashMap<String, Vec<&RawAction>> = HashMap::new();
        let mut kinds: HashMap<String, Vec<i32>> = HashMap::new();
        let mut token_named = Vec::new();
        let mut code = Vec::new();
        let mut code_bytes = 0usize;
        for c in cmds {
            for a in &c.actions {
                if a.kind == SET_TEXT {
                    if let Some(name) = text(&a.text5) {
                        set_text.entry(name.to_lowercase()).or_default().push(a);
                    }
                }
                if a.token_name {
                    token_named.push(a);
                } else if let Some(name) = a.set_name.as_deref() {
                    kinds.entry(name.to_lowercase()).or_default().push(a.kind);
                }
                if a.kind == INLINE_CODE || a.kind == PLUGIN {
                    for t in [&a.text5, &a.text6, &a.text7, &a.text8]
                        .into_iter()
                        .flatten()
                    {
                        code_bytes = code_bytes.saturating_add(t.len());
                        if code_bytes <= MAX_CODE_TEXT {
                            code.push(t.to_lowercase());
                        }
                    }
                }
            }
        }
        Writers {
            set_text,
            kinds,
            token_named,
            code: (code_bytes <= MAX_CODE_TEXT).then_some(code),
            memo: RefCell::new(HashMap::new()),
        }
    }

    /// The text of a code or plugin action has the name `var`. Such an
    /// action can write the variable.
    fn named_by_code(&self, var: &str) -> bool {
        let lower = var.to_lowercase();
        self.code
            .as_ref()
            .is_none_or(|texts| texts.iter().any(|t| t.contains(&lower)))
    }

    /// A type 77 or type 74 action writes the text variable `var` (field
    /// 6).
    fn other_writer(&self, var: &str) -> bool {
        self.kinds.get(&var.to_lowercase()).is_some_and(|k| {
            k.iter()
                .any(|&t| t == SET_TEXT_FROM_INT || t == WAIT_FOR_SPEECH)
        })
    }

    /// Each set action of `var` (without a token in the name) is a type 77
    /// action, and there is one or more.
    fn only_type_77(&self, var: &str) -> bool {
        self.kinds
            .get(&var.to_lowercase())
            .is_some_and(|k| !k.is_empty() && k.iter().all(|&t| t == SET_TEXT_FROM_INT))
            && !self.named_by_code(var)
    }

    /// The grammar of the profile variable `var`, or `None`. A slot
    /// variable and an `EDCP_` variable are not profile variables. A
    /// variable is not known when a set-text with another source sets it,
    /// when its only set is switched off (field 28), when another action
    /// writes it, when the text of a code or
    /// plugin action names it, or when its name fits the pattern of a set
    /// with a token in the variable name. The phrase then stays unresolved,
    /// which is a miss. At most [`MAX_PROFILE_VAR_LOOKUPS`] variables are
    /// checked in one build.
    fn profile_var(&self, patterns: &NamePatterns, var: &str) -> Option<String> {
        if var.starts_with("EDCP_") || slot_kind(var).is_some() {
            return None;
        }
        if let Some(found) = self.memo.borrow().get(var) {
            return found.clone();
        }
        if self.memo.borrow().len() >= MAX_PROFILE_VAR_LOOKUPS {
            return None;
        }
        let found = self.find_profile_var(patterns, var);
        self.memo
            .borrow_mut()
            .insert(var.to_string(), found.clone());
        found
    }

    fn find_profile_var(&self, patterns: &NamePatterns, var: &str) -> Option<String> {
        if self.named_by_code(var) || patterns.fits(var) || self.other_writer(var) {
            return None;
        }
        // The depth of the set is not checked: the only profile variable of
        // the tested profile has its set inside a condition (see `docs/PROFILE_FORMAT.md`).
        match self.set_text.get(&var.to_lowercase()).map(Vec::as_slice) {
            Some([a]) if !a.flag28 && allow::plain(a) => text(&a.text6)
                .filter(|v| !v.contains('{'))
                .map(str::to_string),
            _ => None,
        }
    }
}

/// The reason of each command with a request write when a variable of the
/// request path has a writer that the builder does not model.
const PATH_WRITER: &str = "a variable of the request path has a writer that the program does \
                           not model, the program cannot know where VoiceAttack writes the \
                           request";

/// A variable of the request path (the folder and the file name) has a
/// writer other than a set-text action, anywhere in the profile: a type 74
/// or type 77 action with its name in field 6, or a set with a token in
/// the variable name whose pattern fits it. The builder then cannot know
/// the path of the request file, and it leaves out each command that
/// writes the request. The rule does not ask whether such a writer can run
/// before a write: a writer in any command counts. (Code and plugin actions
/// are not in the rule: the reviewed code of the tested profile sets a
/// path variable in a command that a type 16 action runs after the write.)
fn request_path_writer(writers: &Writers, patterns: &NamePatterns) -> bool {
    allow::write_path_vars()
        .iter()
        .any(|var| patterns.fits(var) || writers.other_writer(var))
}

/// One part of the pattern of a variable name with a token.
#[derive(Debug, Clone, PartialEq, Eq)]
enum NamePart {
    /// Literal text, in lowercase.
    Text(String),
    /// The text of an integer (an optional minus and one or more digits),
    /// or the text of a variable that is not set.
    Integer,
    /// Any text.
    Any,
}

/// The text that VoiceAttack gives for the token of a variable that is
/// not set, in lowercase.
const NOT_SET: &str = "not set";

/// The name patterns of the set actions with a token in the variable name
/// (`token_name`). VoiceAttack puts the value of each token into the name
/// before the set. Thus such a set can write each variable whose name fits
/// its pattern. Names compare without regard to case.
#[derive(Debug, Default)]
pub(super) struct NamePatterns(Vec<Vec<NamePart>>);

impl NamePatterns {
    /// The patterns of the sets with a token in the variable name. A
    /// `{TXT:V}` token gives the text of an integer when each writer of `V`
    /// in the profile is a type 77 action (set a text from an integer).
    /// Each other token gives any text. A set with a token is a writer of
    /// each variable that its pattern fits: the loop removes such a
    /// variable from the integer variables until no pattern fits one of
    /// them.
    ///
    /// The work is bounded, and each bound gives the strict result: more
    /// than [`MAX_NAME_PATTERNS`] distinct names give one pattern that
    /// fits each name; a name longer than [`MAX_PATTERN_NAME`] bytes gives
    /// a pattern that fits each name; more than [`MAX_INTEGER_VARS`]
    /// integer variables, or more than [`MAX_FIXPOINT_STEPS`] rounds of the
    /// loop, give no integer variable (each token gives any text).
    fn new(writers: &Writers) -> Self {
        let mut names: Vec<Option<&str>> = writers
            .token_named
            .iter()
            .map(|a| a.set_name.as_deref())
            .collect();
        names.sort_unstable();
        names.dedup();
        if names.len() > MAX_NAME_PATTERNS {
            return NamePatterns(vec![vec![NamePart::Any]]);
        }
        let mut integers: BTreeSet<String> = names
            .iter()
            .flatten()
            .flat_map(|n| txt_vars(n))
            .filter(|v| writers.only_type_77(v))
            .map(str::to_lowercase)
            .collect();
        if integers.len() > MAX_INTEGER_VARS {
            integers.clear();
        }
        let build = |integers: &BTreeSet<String>| {
            NamePatterns(
                names
                    .iter()
                    .map(|name| match name {
                        Some(n) if n.len() > MAX_PATTERN_NAME => vec![NamePart::Any],
                        _ => name_pattern(*name, |v| integers.contains(&v.to_lowercase())),
                    })
                    .collect(),
            )
        };
        for _ in 0..MAX_FIXPOINT_STEPS {
            let patterns = build(&integers);
            let before = integers.len();
            integers.retain(|v| !patterns.fits(v));
            if integers.len() == before {
                return patterns;
            }
        }
        build(&BTreeSet::new())
    }

    /// `var` fits one or more patterns.
    pub(super) fn fits(&self, var: &str) -> bool {
        let var = var.to_lowercase();
        self.0.iter().any(|p| fits(p, &var))
    }
}

/// The pattern of the variable name `name`. A name that is not a string
/// (`None`) gives any text. A `{` without a `}`, or a token with a `{` in
/// it, gives any text for the rest of the name.
fn name_pattern(name: Option<&str>, integer_only: impl Fn(&str) -> bool) -> Vec<NamePart> {
    let Some(name) = name else {
        return vec![NamePart::Any];
    };
    let mut parts = Vec::new();
    let mut rest = name;
    while let Some(open) = rest.find('{') {
        if open > 0 {
            parts.push(NamePart::Text(rest[..open].to_lowercase()));
        }
        let token = &rest[open + 1..];
        let Some(close) = token.find('}') else {
            if parts.last() != Some(&NamePart::Any) {
                parts.push(NamePart::Any);
            }
            return parts;
        };
        let inner = &token[..close];
        if inner.contains('{') {
            if parts.last() != Some(&NamePart::Any) {
                parts.push(NamePart::Any);
            }
            return parts;
        }
        let integer = inner
            .strip_prefix("TXT:")
            .is_some_and(|v| !v.is_empty() && integer_only(v));
        if integer {
            parts.push(NamePart::Integer);
        } else if parts.last() != Some(&NamePart::Any) {
            // Two parts of any text are one part.
            parts.push(NamePart::Any);
        }
        rest = &token[close + 1..];
    }
    if !rest.is_empty() {
        parts.push(NamePart::Text(rest.to_lowercase()));
    }
    parts
}

/// The most distinct names of sets with a token in the variable name. The
/// tested profile has 4.
const MAX_NAME_PATTERNS: usize = 32;

/// The longest name (in bytes) of a set with a token that gives a pattern
/// with parts. The tested profile has names of at most 35 bytes.
const MAX_PATTERN_NAME: usize = 256;

/// The most integer variables that the patterns check.
const MAX_INTEGER_VARS: usize = 16;

/// The most rounds of the loop of `NamePatterns::new`.
const MAX_FIXPOINT_STEPS: usize = 4;

/// The largest number of parts of a pattern that the match reads. A
/// pattern with more parts fits each name.
const MAX_PATTERN_PARTS: usize = 64;

/// The longest variable name (in bytes) that the match reads. A longer
/// name fits each pattern.
const MAX_MATCHED_NAME: usize = 256;

/// The lowercase `name` fits `pattern`. The match keeps the set of the
/// name positions that the parts so far can reach. Thus the work is at most
/// the number of parts times the square of the name length, with no
/// backtracking. Above the limits, the result is "fits", which is the
/// strict direction.
fn fits(pattern: &[NamePart], name: &str) -> bool {
    if pattern.len() > MAX_PATTERN_PARTS || name.len() > MAX_MATCHED_NAME {
        return true;
    }
    let bytes = name.as_bytes();
    let n = bytes.len();
    let mut reach = vec![false; n + 1];
    reach[0] = true;
    for part in pattern {
        let mut next = vec![false; n + 1];
        match part {
            NamePart::Text(t) => {
                let t = t.as_bytes();
                for i in (0..=n).filter(|&i| reach[i]) {
                    if bytes[i..].starts_with(t) {
                        next[i + t.len()] = true;
                    }
                }
            }
            NamePart::Any => {
                // Each position at or after a reached position. A literal
                // part after it starts only where its UTF-8 text starts, so
                // a position inside a character gives no match.
                let mut seen = false;
                for i in 0..=n {
                    seen |= reach[i];
                    next[i] = seen;
                }
            }
            NamePart::Integer => {
                let not_set = NOT_SET.as_bytes();
                for i in (0..=n).filter(|&i| reach[i]) {
                    if bytes[i..].starts_with(not_set) {
                        next[i + not_set.len()] = true;
                    }
                }
                // An optional minus and one or more digits: one pass that
                // keeps a run of digits that started at a reached position.
                let mut run = false;
                for j in 0..n {
                    if bytes[j].is_ascii_digit() {
                        let start = reach[j] || (j > 0 && bytes[j - 1] == b'-' && reach[j - 1]);
                        run |= start;
                        next[j + 1] |= run;
                    } else {
                        run = false;
                    }
                }
            }
        }
        reach = next;
        if !reach.contains(&true) {
            return false;
        }
    }
    reach[n]
}

/// The phrase of a number record with the grammar of each profile
/// variable in it passes the number check.
fn number_phrase_with_vars(phrase: &str, ctx: &Context) -> bool {
    let mut full = phrase.to_string();
    for var in txt_vars(phrase) {
        if let Some(grammar) = ctx.writers.profile_var(&ctx.patterns, var) {
            full = full.replace(&format!("{{TXT:{var}}}"), &format!(" {grammar} "));
        }
    }
    !number_text_outside_ranges(&full)
}

#[cfg(test)]
mod tests {
    use super::super::record::walk;
    use super::super::test_support::*;
    use super::*;

    fn run(cmds: Vec<Vec<u8>>) -> Result<Extracted, BuildError> {
        extract(&walk(&profile(cmds)).unwrap(), &[])
    }

    fn reason_of(out: &Extracted, phrase: &str) -> (LeftOutClass, String) {
        let l = out.left_out.iter().find(|l| l.phrase == phrase).unwrap();
        (l.class, l.reason.clone())
    }

    #[test]
    fn test_placeholders_follow_the_regex() {
        assert_eq!(
            placeholders("a{TXT:x}b{TXTNUM:n}{CLIP}{CMD}{OTHER}{TXT:}"),
            [Place::Txt("x"), Place::Num("n"), Place::Clip, Place::Cmd]
        );
        // `[^}]+` takes an inner `{`.
        assert_eq!(placeholders("{TXT:a{b}"), [Place::Txt("a{b")]);
        assert_eq!(placeholders("{X{CMD}"), [Place::Cmd]);
        assert!(placeholders("{TXT:open").is_empty());
    }

    #[test]
    fn test_classify() {
        assert_eq!(classify("Plain"), (CommandClass::Fixed, None));
        assert_eq!(
            classify("A:{TXT:Group12}"),
            (CommandClass::DynamicName, Some("Group12".into()))
        );
        assert_eq!(
            classify("A:{TXT:v}{CLIP}"),
            (CommandClass::DynamicValue, Some("v,CLIP".into()))
        );
        assert_eq!(
            classify("{CMD}"),
            (CommandClass::DynamicName, Some("CMD".into()))
        );
    }

    #[test]
    fn test_full_command_record() {
        let out = run(vec![command(
            "go [1..9]",
            vec![
                sound(),
                set_text("LastCMD", "{CMD}"),
                set_text("Dist", "{TXTNUM:LastCMD}"),
                write("Go={TXT:Dist}"),
            ],
            0,
            None,
        )])
        .unwrap();
        let r = &out.commands[0];
        assert_eq!(r.token_template, "Go={TXT:Dist}");
        assert_eq!(r.class, CommandClass::DynamicValue);
        assert_eq!(r.number_vars, ["Dist"]);
        assert_eq!(r.category.as_deref(), Some("cat"));
        assert!(out.left_out.is_empty());
    }

    #[test]
    fn test_constant() {
        let out = run(vec![command(
            "sail to the far island",
            vec![
                set_text("SystemName", "Far Island"),
                write("Sail:{TXT:SystemName}"),
            ],
            0,
            None,
        )])
        .unwrap();
        assert_eq!(
            out.commands[0].constants,
            HashMap::from([("SystemName".to_string(), "Far Island".to_string())])
        );
    }

    #[test]
    fn test_left_out_classes() {
        let out = run(vec![
            command("none", vec![sound(), write_other_file("x")], 0, None),
            command("two", vec![write("A"), write("B")], 0, None),
            command(
                "gated",
                vec![condition("v", "x", 6), write("C"), end_condition()],
                0,
                None,
            ),
            command("keys", vec![write("D"), key_press()], 0, None),
            command("lonely", vec![write("E")], 2, Some("nobody")),
            command("odd", vec![plain_action(71), write("F")], 0, None),
        ])
        .unwrap();
        assert!(out.commands.is_empty());
        let classes: Vec<_> = out.left_out.iter().map(|l| l.class).collect();
        assert_eq!(
            classes,
            [
                LeftOutClass::NoWrite,
                LeftOutClass::MultiWrite,
                LeftOutClass::Maintenance,
                LeftOutClass::Maintenance,
                LeftOutClass::Orphan,
                LeftOutClass::NotUnderstood,
            ]
        );
        assert_eq!(out.left_out[1].tokens, ["A", "B"]);
        assert_eq!(out.left_out[5].tokens, ["F"]);
    }

    /// Each left-out command that VoiceAttack recognises by voice gives a
    /// blocking phrase. A command that is switched off, has no voice
    /// trigger, has an internal `((...))` phrase, or is an orphan suffix
    /// gives none.
    #[test]
    fn test_blocking_phrases_of_the_left_out_commands() {
        let switched = |phrase: &str, field: usize, value: Vec<u8>| {
            let mut f = command_fields(phrase, vec![sound()], 0, None);
            f[field] = value;
            record(f)
        };
        let out = run(vec![
            command("none", vec![sound(), write_other_file("x")], 0, None),
            command("two", vec![write("A"), write("B")], 0, None),
            command("keys", vec![write("D"), key_press()], 0, None),
            command("lonely", vec![write("E")], 2, Some("nobody")),
            command("odd", vec![plain_action(71), write("F")], 0, None),
            command("((inner work))", vec![sound()], 0, None),
            command("  ", vec![sound()], 0, None),
            switched("quiet", 15, vec![1, 0]),
            switched("off", 4, vec![0]),
            command("kept", vec![write("G")], 0, None),
        ])
        .unwrap();
        let blocking: Vec<(&str, LeftOutClass)> = out
            .blocking
            .iter()
            .map(|b| (b.phrase.as_str(), b.class))
            .collect();
        assert_eq!(
            blocking,
            [
                ("none", LeftOutClass::NoWrite),
                ("two", LeftOutClass::MultiWrite),
                ("keys", LeftOutClass::Maintenance),
                ("odd", LeftOutClass::NotUnderstood),
            ]
        );
        assert!(out
            .blocking
            .iter()
            .all(|b| b.command_type == CommandType::Full && b.composite_group.is_none()));
        // The commands are the same with and without the blocking phrases.
        assert_eq!(out.commands.len(), 1);
    }

    /// A left-out prefix or suffix with a group gives a blocking phrase
    /// with its group.
    #[test]
    fn test_a_left_out_composite_part_blocks_with_its_group() {
        let out = run(vec![
            command(
                "dock [buy; sell]",
                vec![set_text("Kind", "Dock:")],
                1,
                Some("g"),
            ),
            // A prefix that writes a token is not understood.
            command("pier [buy; sell]", vec![write("A")], 1, Some("g")),
            // A suffix of another shape is not understood.
            command(
                "cork",
                vec![plain_action(99), write("{TXT:Kind}Cork")],
                2,
                Some("g"),
            ),
        ])
        .unwrap();
        let parts: Vec<(&str, CommandType, Option<&str>)> = out
            .blocking
            .iter()
            .map(|b| {
                (
                    b.phrase.as_str(),
                    b.command_type,
                    b.composite_group.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            parts,
            [
                ("pier [buy; sell]", CommandType::Prefix, Some("g")),
                ("cork", CommandType::Suffix, Some("g")),
            ]
        );
    }

    #[test]
    fn test_old_class_wins_over_the_whitelist() {
        // A command with two writes and a type that the whitelist does not
        // allow keeps the class of the script.
        let out = run(vec![command(
            "two",
            vec![plain_action(71), write("A"), write("B")],
            0,
            None,
        )])
        .unwrap();
        assert_eq!(out.left_out[0].class, LeftOutClass::MultiWrite);
    }

    fn suffix(actions: Vec<Vec<u8>>) -> Vec<u8> {
        command("[now;] flag", actions, 2, Some("grp"))
    }

    fn prefix() -> Vec<u8> {
        command(
            "fly the [blue; red]",
            vec![set_text("Size", "Big:")],
            1,
            Some("grp"),
        )
    }

    fn good_suffix() -> Vec<Vec<u8>> {
        vec![
            sound(),
            condition("{PREFIX}", "blue", 6),
            set_text("Colour", "Blue"),
            end_condition(),
            condition("{PREFIX}", "red", 6),
            set_text("Colour", "Red"),
            end_condition(),
            condition("Colour", "", 9),
            say("missing"),
            else_branch(),
            write("{TXT:Colour}{TXT:Size}Flag"),
            end_condition(),
        ]
    }

    #[test]
    fn test_composite_records() {
        let out = run(vec![prefix(), suffix(good_suffix())]).unwrap();
        let prefix = &out.commands[0];
        assert_eq!(prefix.command_type, CommandType::Prefix);
        assert_eq!(prefix.sets.get("Size").map(String::as_str), Some("Big:"));
        let s = &out.commands[1];
        assert_eq!(s.command_type, CommandType::Suffix);
        assert_eq!(s.prefix_rules.len(), 2);
        assert_eq!(s.prefix_rules[1].contains, "red");
        assert_eq!(s.composite_group.as_deref(), Some("grp"));
    }

    #[test]
    fn test_unknown_suffix_shape_leaves_the_suffix_out() {
        let mut acts = good_suffix();
        // A second action in a {PREFIX} block.
        acts.insert(3, sound());
        let out = run(vec![prefix(), suffix(acts)]).unwrap();
        assert_eq!(
            reason_of(&out, "[now;] flag"),
            (
                LeftOutClass::NotUnderstood,
                "the suffix has a {PREFIX} block of an unknown shape".into()
            )
        );
        let mut acts = good_suffix();
        // A file write before the else branch.
        acts.insert(8, write_other_file("X"));
        let out = run(vec![prefix(), suffix(acts)]).unwrap();
        assert_eq!(
            reason_of(&out, "[now;] flag"),
            (
                LeftOutClass::NotUnderstood,
                "the suffix has a condition block of an unknown shape".into()
            )
        );
        // The prefix stays in the map.
        assert_eq!(out.commands.len(), 1);
    }

    #[test]
    fn test_prefix_that_writes_is_left_out() {
        let out = run(vec![command("p", vec![write("A")], 1, Some("g"))]).unwrap();
        assert_eq!(
            reason_of(&out, "p"),
            (
                LeftOutClass::NotUnderstood,
                "the prefix writes a request token".into()
            )
        );
        assert_eq!(out.left_out[0].tokens, ["A"]);
    }

    #[test]
    fn test_phrase_that_is_a_category_is_left_out() {
        let out = run(vec![command("cat", vec![write("A")], 0, None)]).unwrap();
        assert_eq!(
            reason_of(&out, "cat"),
            (
                LeftOutClass::NotUnderstood,
                "the phrase is also the name of a category".into()
            )
        );
    }

    #[test]
    fn test_command_without_a_phrase_is_left_out() {
        let mut f = command_fields("x", vec![write("A")], 0, None);
        f[1] = null_str();
        let out = run(vec![record(f)]).unwrap();
        assert_eq!(
            reason_of(&out, ""),
            (
                LeftOutClass::NotUnderstood,
                "the command has no phrase".into()
            )
        );
    }

    #[test]
    fn test_profile_vars() {
        let out = run(vec![
            command("{TXT:Places} now", vec![write("P:{CMD}")], 0, None),
            command("init", vec![set_text("Places", "[a; b]")], 0, None),
            command("{TXT:System3} go", vec![write("S:{CMD}")], 0, None),
        ])
        .unwrap();
        assert_eq!(
            out.profile_vars,
            HashMap::from([("Places".to_string(), "[a; b]".to_string())])
        );
    }

    #[test]
    fn test_the_name_match_has_no_backtracking() {
        let lit = |t: &str| NamePart::Text(t.to_string());
        // 63 parts: any text and "a" in turns, then "c". The name has 256
        // bytes and no "c": a backtracking match takes very long.
        let mut pattern = Vec::new();
        for _ in 0..31 {
            pattern.push(NamePart::Any);
            pattern.push(lit("a"));
        }
        pattern.push(lit("c"));
        let name = "a".repeat(255) + "b";
        let started = std::time::Instant::now();
        assert!(!fits(&pattern, &name));
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
        // The same with integer parts.
        let pattern: Vec<NamePart> = (0..63)
            .map(|k| {
                if k % 2 == 0 {
                    NamePart::Integer
                } else {
                    NamePart::Any
                }
            })
            .chain([lit("c")])
            .collect();
        let name = "1".repeat(255) + "b";
        let started = std::time::Instant::now();
        assert!(!fits(&pattern, &name));
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
    }

    #[test]
    fn test_a_pattern_or_a_name_above_the_limits_fits() {
        let many = vec![NamePart::Text("x".into()); MAX_PATTERN_PARTS + 1];
        assert!(fits(&many, "lamp"));
        let one = [NamePart::Text("x".into())];
        assert!(!fits(&one, "lamp"));
        assert!(fits(&one, &"l".repeat(MAX_MATCHED_NAME + 1)));
        // The parts of a name with many tokens: two parts of any text are
        // one part.
        let name = "{TXT:a}{TXT:b}x{TXT:c}{TXT:d";
        assert_eq!(
            name_pattern(Some(name), |_| false),
            [NamePart::Any, NamePart::Text("x".into()), NamePart::Any]
        );
    }

    #[test]
    fn test_the_name_match_reads_each_part() {
        let lit = |t: &str| NamePart::Text(t.to_string());
        let int = NamePart::Integer;
        let any = NamePart::Any;
        let p = [lit("lamp"), int.clone()];
        for yes in ["lamp1", "lamp-12", "lampnot set"] {
            assert!(fits(&p, yes), "{yes}");
        }
        for no in ["lamp", "lamp-", "lampx", "lamp1x", "lamp--1", "buoy1"] {
            assert!(!fits(&p, no), "{no}");
        }
        let p = [lit("lamp"), int.clone(), lit("words")];
        assert!(fits(&p, "lamp7words"));
        assert!(!fits(&p, "lampwords"));
        let p = [lit("a"), any.clone(), lit("b"), int, any];
        assert!(fits(&p, "axxb3yy"));
        assert!(fits(&p, "ab-3"));
        assert!(!fits(&p, "axxbyy"));
        assert!(fits(&[], ""));
        assert!(!fits(&[], "x"));
        // Text that is not ASCII.
        assert!(fits(&[lit("ä"), NamePart::Any, lit("ö")], "äüö"));
    }
}
