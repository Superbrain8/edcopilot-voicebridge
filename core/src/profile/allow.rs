//! The action table and the whitelist of `docs/PROFILE_FORMAT.md`
//! (section "The whitelist").
//!
//! The builder puts a command into the map only when each action is
//! allowed at its position. Otherwise the builder leaves the command out
//! with a reason. This gives a miss, never a wrong match. Each function
//! returns `Err(reason)` for a command that the builder must leave out.
//!
//! An action with field 28 = 1 is probably switched off. The builder
//! cannot know what VoiceAttack then does. Thus such an action at or
//! before the write leaves the command out. After the write, it counts as
//! present for the rules after the write.
//!
//! After the write, the token is already in the request file. Thus the
//! builder also allows condition and set actions after the write.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use super::extract::{placeholders, text, txt_vars, Place, NUM_OF_CMD, REQUEST_PATH};
use super::record::{
    RawAction, RawCommand, CLIPBOARD, COND_BEGIN, COND_ELSE, COND_ELSE_IF, COND_END, EXECUTE,
    INLINE_CODE, KNOWN_OPERATORS, LOOP_BEGIN, LOOP_END, PLUGIN, SET_INT, SET_TEXT, WRITE_FILE,
};
use crate::command_map::{Command, CommandType, PrefixRule};
use crate::dynamic_vocab::{holds_one_plain_name, slot_kind};
use crate::grammar::MAX_GROUP_DEPTH;

const OP_CONTAINS: i32 = 6;
const OP_NOT_SET: i32 = 9;

/// Whitelist item 1: types that cannot change a variable or the write.
/// Type 24 (the clipboard) is allowed only when the token has no `{CLIP}`.
const INERT: [i32; 6] = [2, 13, 14, 23, 28, 34];
/// Whitelist item 2: types that do not change a text variable.
const OTHER_VARIABLE: [i32; 4] = [18, 36, 37, 38];
/// Types that do something that the program does not do, but cannot
/// change the token. Allowed only after the write.
const SIDE_EFFECT: [i32; 8] = [0, 3, 4, 15, 50, 51, 70, 76];
/// Condition actions and set actions. Allowed after the write: the token
/// is already in the file.
const AFTER_WRITE_EXTRA: [i32; 11] = [19, 20, 29, 63, 21, 36, 37, 38, 18, 24, 77];
/// The action types whose unread fields must have the tested values (the
/// type 37 only in the system-slot parse).
const MODELLED: [i32; 7] = [
    SET_TEXT,
    WRITE_FILE,
    COND_BEGIN,
    COND_END,
    COND_ELSE,
    COND_ELSE_IF,
    EXECUTE,
];
/// The files that a command that a type 16 action runs may write: the
/// file names of the tested profile, other than the request file. The
/// builder compares them ASCII case-insensitive. A person adds a name only
/// after a check of the commands that write it.
const OTHER_FILES: [&str; 3] = [
    "EDCoPilot.VApath.txt",
    "LaunchEDCoPilot.bat",
    "EDCoPilot.VAPversion.txt",
];
/// The most distinct names that an execute action by name can resolve in
/// one build. The tested profile has 17. An execute action with a name
/// beyond the limit counts as a target that the builder cannot find.
pub(super) const MAX_NAME_TARGETS: usize = 64;
/// The most words of a name that an execute action by name can resolve.
const MAX_NAME_WORDS: usize = 64;
/// The longest chain of execute actions that the target check follows. A
/// longer chain counts as a chain that can write a request token.
const MAX_CHAIN: usize = 32;

/// The SHA-256 (lowercase hex) of the code of each type 71 action that a
/// person read in full. The hash input is the fields 5 (the code text), 6
/// (the assembly list) and 11, each with its length first. Type 71 code in
/// a command that a type 16 action runs is allowed only with a hash of
/// this table. Type 71 directly in a command of the map is never allowed.
pub(super) const REVIEWED_CODE: [&str; 2] = [
    // Reads two registry values (read access only) and sets two text variables. One is the path variable that later request writes of VoiceAttack read. The caller writes its token before the chain runs, and no command of the chain writes a request token, so the token of the caller does not change. No file write, no process start, no command execution.
    "d0f4b791def6824cb1e6d62ee521fd8e84dd441039cb940f85382c2fdd6706dd",
    // Parses two text variables that hold the voice command files and sets several EDCP_ text variables for each entry. It writes up to four log lines. No file write, no process start, no command execution.
    "9b8d5e9ee41cd8353cc5a8dbcc128e7bdacc87c79a32df4fb94bffe73a8348fc",
];

/// The action types of the audit, with a short name for a person.
const TABLE: [(i32, &str); 35] = [
    (0, "press keys"),
    (2, "pause"),
    (3, "run an application"),
    (4, "act on a process"),
    (13, "say a text"),
    (14, "play a sound"),
    (15, "change the profile"),
    (16, "execute a command"),
    (18, "set a small integer"),
    (19, "begin a condition"),
    (20, "end a condition"),
    (21, "set a text variable"),
    (22, "plugin function"),
    (23, "write to the event log"),
    (24, "set the clipboard"),
    (28, "comment"),
    (29, "else"),
    (30, "begin a loop"),
    (31, "end a loop"),
    (34, "window action"),
    (36, "set a boolean"),
    (37, "set an integer"),
    (38, "set a decimal"),
    (44, "write to a file"),
    (50, "start listening"),
    (51, "stop listening"),
    (57, "stop all commands"),
    (63, "else if"),
    (69, "purpose not known"),
    (70, "set an audio level"),
    (71, "inline code"),
    (74, "wait for a spoken response"),
    (75, "exit a loop"),
    (76, "reload the commands"),
    (77, "set a text from an integer"),
];

/// The short name of an action type, or "not known" when the table does
/// not list the type.
pub(super) fn type_name(kind: i32) -> &'static str {
    TABLE
        .iter()
        .find(|(k, _)| *k == kind)
        .map_or("not known", |(_, name)| name)
}

fn in_table(kind: i32) -> bool {
    TABLE.iter().any(|(k, _)| *k == kind)
}

fn not_allowed(kind: i32, place: &str) -> String {
    format!("action type {kind} ({}) {place}", type_name(kind))
}

const TOKEN_VARIABLE: &str =
    "the command sets a part of its token in a way that the program does not model";
const PREFIX_SET: &str = "the prefix sets a variable in a way that the program does not model";
const TARGET_UNKNOWN: &str = "action type 16 (execute a command) after the write: the program \
                              cannot find the command that it runs";
const TARGET_WRITES: &str = "action type 16 (execute a command) after the write: the command \
                             that it runs can write a request token";
const TARGET_CODE: &str = "action type 16 (execute a command) after the write: the command \
                           that it runs has code that is not reviewed";
const TARGET_SETTING: &str = "action type 16 (execute a command) after the write: the command \
                              that it runs has a setting that the program does not know";
const SWITCHED_OFF: &str = "an action before the write is switched off in the profile, the \
                            program cannot know what VoiceAttack does";
const PREFIX_SWITCHED_OFF: &str = "an action of the prefix is switched off in the profile, the \
                                   program cannot know what VoiceAttack does";
const WRITE_PATH: &str = "the command changes the path of the request file";
const SLOT_VARIABLE: &str =
    "the command sets a variable of the system-slot parse outside the parse";
const TOKEN_NAME: &str = "a set action at or before the write has a variable name with a token, \
                          the program cannot know which variable it sets";
const SLOT_TEXT: &str =
    "the token takes the whole text of a phrase slot whose file can hold more than one name";
const NUMBER_SUFFIX: &str = "the number of a composite command can take digits of the prefix";
const RULE_VAR_SET: &str =
    "the suffix sets a variable of its prefix rules outside the {PREFIX} blocks";
pub(super) const UNKNOWN_SETTING: &str = "an action has a setting that the program does not know";

/// A set-text action with the literal source (field 13 = 0) and no file,
/// copy, or replace field (whitelist item 3).
pub(super) fn plain(a: &RawAction) -> bool {
    a.kind == SET_TEXT
        && a.int13 == 0
        && a.int12 == 0
        && text(&a.text7).is_none()
        && text(&a.text8).is_none()
        && text(&a.text14).is_none()
        && text(&a.text15).is_none()
}

/// A condition that compares a text with a value: no second variable and
/// no compound condition.
fn simple_compare(a: &RawAction) -> bool {
    a.int23 == 1 && a.int20 == 0 && a.int21 == 0 && text(&a.text22).is_none() && !a.compound
}

fn is_inert(kind: i32, clip: bool) -> bool {
    INERT.contains(&kind) || (kind == CLIPBOARD && !clip)
}

fn lower(name: &Option<String>) -> String {
    text(name).unwrap_or("").to_lowercase()
}

/// The lowercase names of the variables that the request path reads.
pub(super) fn write_path_vars() -> Vec<String> {
    txt_vars(REQUEST_PATH)
        .into_iter()
        .map(str::to_lowercase)
        .collect()
}

/// A set-text action that sets one of the variables of the request path.
fn sets_write_path(a: &RawAction, path_vars: &[String]) -> bool {
    a.kind == SET_TEXT && path_vars.contains(&lower(&a.text5))
}

/// An action of a modelled type with a setting that the tested profile
/// does not have.
fn unknown_setting(a: &RawAction) -> bool {
    MODELLED.contains(&a.kind) && a.unknown_setting.is_some()
}

/// Every condition uses an operator that the builder knows.
fn check_operators(actions: &[RawAction]) -> Result<(), String> {
    for a in actions {
        if (a.kind == COND_BEGIN || a.kind == COND_ELSE_IF)
            && !KNOWN_OPERATORS.contains(&a.operator)
        {
            return Err(format!(
                "a condition uses the operator {}, which the program does not know",
                a.operator
            ));
        }
    }
    Ok(())
}

/// The block depth before each action. A condition and a loop begin a
/// block. The depth can go below zero in a broken command.
fn depths(actions: &[RawAction]) -> Vec<i32> {
    let mut d = 0;
    actions
        .iter()
        .map(|a| {
            let before = d;
            match a.kind {
                COND_BEGIN | LOOP_BEGIN => d += 1,
                COND_END | LOOP_END => d -= 1,
                _ => {}
            }
            before
        })
        .collect()
}

/// Two names of `names` differ only in case.
fn case_conflict<'s>(names: impl IntoIterator<Item = &'s str>) -> bool {
    let names: Vec<&str> = names.into_iter().collect();
    names.iter().enumerate().any(|(i, a)| {
        names[i + 1..]
            .iter()
            .any(|b| a != b && a.eq_ignore_ascii_case(b))
    })
}

// ---------------------------------------------------------------------
// Execute a command (type 16)
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TargetProblem {
    Unknown,
    Writes,
    Code,
    Setting,
}

/// The commands by id and by phrase, to find the target of an execute
/// action, and the result of the target check of each target.
pub(super) struct Index<'a> {
    /// `None` when two commands have the same id.
    by_id: HashMap<[u8; 16], Option<&'a RawCommand>>,
    /// The commands of each exact phrase.
    by_phrase: HashMap<&'a str, Vec<&'a RawCommand>>,
    /// The number of commands of each phrase in its normal form (see
    /// [`words_of`]).
    by_normal: HashMap<String, usize>,
    /// The lowercase file names that a set-text action of the profile gives
    /// to a variable of the request path.
    request_files: HashSet<String>,
    /// The hashes of the reviewed code.
    reviewed: &'a [&'a str],
    /// The result of the target walk of each command, by its offset: the
    /// most execute steps below it, or the problem.
    memo: RefCell<HashMap<usize, Result<usize, TargetProblem>>>,
    /// Each command whose phrase can give other names than the normal form
    /// of its text: a phrase with a group, a list, a variable, or a bracket
    /// that the parser cannot read. The parts are read once. `None`: the
    /// phrase does not parse, and it can give each name.
    complex: Vec<(&'a RawCommand, Option<Vec<Vec<Part>>>)>,
    /// For each name of an execute action: another phrase can give it. At
    /// most [`MAX_NAME_TARGETS`] names.
    names: RefCell<HashMap<String, bool>>,
}

/// The file name part of a path: the text after the last `\` or `/`.
fn file_part(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

impl<'a> Index<'a> {
    pub(super) fn new(commands: &'a [RawCommand], reviewed: &'a [&'a str]) -> Index<'a> {
        let mut by_id: HashMap<[u8; 16], Option<&RawCommand>> = HashMap::new();
        let mut by_phrase: HashMap<&str, Vec<&RawCommand>> = HashMap::new();
        let mut by_normal: HashMap<String, usize> = HashMap::new();
        let mut request_files = HashSet::new();
        let mut complex = Vec::new();
        let path_vars = write_path_vars();
        for c in commands {
            by_id
                .entry(c.id)
                .and_modify(|e| *e = None)
                .or_insert(Some(c));
            if let Some(p) = text(&c.phrase) {
                by_phrase.entry(p).or_default().push(c);
                *by_normal.entry(words_of(p).join(" ")).or_default() += 1;
                if p.contains(['[', ']', '{', '}', ';']) {
                    complex.push((c, parse_parts(p, 0).map(|(parts, _)| parts)));
                }
            }
            for a in &c.actions {
                if sets_write_path(a, &path_vars) {
                    if let Some(value) = text(&a.text6) {
                        request_files.insert(file_part(value).to_lowercase());
                    }
                }
            }
        }
        Index {
            by_id,
            by_phrase,
            by_normal,
            request_files,
            reviewed,
            memo: RefCell::new(HashMap::new()),
            complex,
            names: RefCell::new(HashMap::new()),
        }
    }

    /// The one command with the exact phrase `name`. `None` for a name
    /// with a variable, a name without single spaces between its words, a
    /// name of more than [`MAX_NAME_WORDS`] words, a name whose normal form
    /// (see [`words_of`]) more than one phrase has, a name that the phrase
    /// of another command can also give (see [`phrase_can_give`]), or a new
    /// name after [`MAX_NAME_TARGETS`] names. A phrase without a group, a
    /// list or a variable gives only its normal form: the count of the
    /// normal forms covers it.
    fn by_name(&self, name: &str) -> Option<&'a RawCommand> {
        let words = words_of(name);
        let spaced = name.split_whitespace().collect::<Vec<_>>().join(" ") == name;
        if name.contains('{')
            || !spaced
            || words.len() > MAX_NAME_WORDS
            || self.by_normal.get(&words.join(" ")) != Some(&1)
        {
            return None;
        }
        let one = match self.by_phrase.get(name).map(Vec::as_slice) {
            Some([one]) => *one,
            _ => return None,
        };
        if let Some(&shared) = self.names.borrow().get(name) {
            return (!shared).then_some(one);
        }
        if self.names.borrow().len() >= MAX_NAME_TARGETS {
            return None;
        }
        let shared = self.complex.iter().any(|(c, parts)| {
            !std::ptr::eq(*c, one) && parts.as_ref().is_none_or(|p| parts_can_give(p, &words))
        });
        self.names.borrow_mut().insert(name.to_string(), shared);
        (!shared).then_some(one)
    }

    /// The target of an execute action. The mode (field 3) chooses: "by
    /// name" needs the zero id and the exact name (field 6). "By id" needs
    /// an id (field 5) that one command has. A name next to the id must
    /// name the same command. `None` for each other case.
    fn resolve(&self, a: &RawAction) -> Option<&'a RawCommand> {
        let id = match text(&a.text5) {
            Some(t) => parse_guid(t)?,
            None => [0; 16],
        };
        match a.exec_by_name? {
            true => {
                if id != [0; 16] {
                    return None;
                }
                self.by_name(text(&a.text6)?)
            }
            false => {
                if id == [0; 16] {
                    return None;
                }
                let target = self.by_id.get(&id).copied().flatten()?;
                match text(&a.text6) {
                    Some(name) if !self.by_name(name).is_some_and(|n| std::ptr::eq(n, target)) => {
                        None
                    }
                    _ => Some(target),
                }
            }
        }
    }

    /// A file write in a target that can reach the request file. Only a
    /// known file name of [`OTHER_FILES`] is another file: the path text
    /// after the last placeholder must be exactly `\` and that name
    /// (ASCII case-insensitive), and the profile must not give that name to
    /// a variable of the request path. Each other path can reach the
    /// request file: Windows also opens a file through `.`, `..`, a short
    /// 8.3 name, a name with a point or a space at the end, or a stream.
    fn can_write_request(&self, a: &RawAction) -> bool {
        if a.kind != WRITE_FILE {
            return false;
        }
        let Some(path) = text(&a.text6) else {
            return true;
        };
        let after = path.rfind('}').map_or(path, |k| &path[k + 1..]);
        let known = after.strip_prefix('\\').and_then(|name| {
            OTHER_FILES
                .iter()
                .find(|known| known.eq_ignore_ascii_case(name))
        });
        match known {
            Some(name) => self.request_files.contains(&name.to_lowercase()),
            None => true,
        }
    }

    /// The target check: the target writes no request token, also through
    /// the commands that it executes, and the longest chain of execute
    /// steps below it has at most [`MAX_CHAIN`] steps.
    fn check_target(&self, target: &'a RawCommand) -> Result<(), TargetProblem> {
        match self.height(target)? {
            steps if steps > MAX_CHAIN => Err(TargetProblem::Writes),
            _ => Ok(()),
        }
    }

    /// The problem of the actions of one command, without the commands
    /// that it executes, and the commands that it executes. A target with
    /// an unknown command setting, a plugin, a file write that can reach
    /// the request file, an action type that the table does not list, a
    /// write or execute action with a setting that the tested profile does
    /// not have, or code that is not reviewed, can write a request token.
    fn own_problem(&self, cmd: &'a RawCommand) -> Result<Vec<&'a RawCommand>, TargetProblem> {
        if cmd.unknown_setting.is_some() {
            return Err(TargetProblem::Setting);
        }
        let mut next = Vec::new();
        for a in &cmd.actions {
            let bad_setting =
                (a.kind == WRITE_FILE || a.kind == EXECUTE) && a.unknown_setting.is_some();
            if self.can_write_request(a) || a.kind == PLUGIN || !in_table(a.kind) || bad_setting {
                return Err(TargetProblem::Writes);
            }
            if a.kind == INLINE_CODE && !self.reviewed_code(a) {
                return Err(TargetProblem::Code);
            }
            if a.kind == EXECUTE {
                next.push(self.resolve(a).ok_or(TargetProblem::Unknown)?);
            }
        }
        Ok(next)
    }

    /// The most execute steps below `target`, or the problem of a command
    /// that it reaches. The walk keeps the result of each command for the
    /// whole build, so each command is walked once in a build. It uses its
    /// own stack (no recursion).
    ///
    /// Commands that execute each other in a loop are one group (a strongly
    /// connected component, found with the algorithm of Tarjan). A group
    /// counts one step for each command in it after the first, plus the
    /// steps below it: this is not less than the longest chain without a
    /// repeated command. A count above [`MAX_CHAIN`] is a problem.
    fn height(&self, target: &'a RawCommand) -> Result<usize, TargetProblem> {
        if let Some(r) = self.memo.borrow().get(&target.offset) {
            return *r;
        }
        let mut walk = Walk::default();
        let result = self.walk_from(target, &mut walk);
        if let Err(p) = result {
            // Each command on the stacks reaches the command with the
            // problem.
            let mut memo = self.memo.borrow_mut();
            for frame in &walk.frames {
                memo.insert(frame.cmd.offset, Err(p));
            }
            for cmd in &walk.group {
                memo.insert(cmd.offset, Err(p));
            }
        }
        result
    }

    /// The steps of [`Index::height`].
    fn walk_from(
        &self,
        target: &'a RawCommand,
        walk: &mut Walk<'a>,
    ) -> Result<usize, TargetProblem> {
        self.enter(target, walk)?;
        while let Some(frame) = walk.frames.last_mut() {
            if frame.k < frame.next.len() {
                let child = frame.next[frame.k];
                frame.k += 1;
                let known = self.memo.borrow().get(&child.offset).copied();
                match known {
                    Some(Ok(h)) => frame.below = frame.below.max(h + 1),
                    Some(Err(p)) => return Err(p),
                    None => match walk.index.get(&child.offset) {
                        // A command of the group on the stack: a loop.
                        Some(&at) => frame.low = frame.low.min(at),
                        None => self.enter(child, walk)?,
                    },
                }
                continue;
            }
            let frame = walk.frames.pop().expect("the frame is on the stack");
            let mut below = frame.below;
            if frame.low == frame.at {
                // The first command of a group: the group is complete.
                let at = walk
                    .group
                    .iter()
                    .rposition(|c| c.offset == frame.cmd.offset)
                    .expect("the command is in the group stack");
                let members: Vec<&'a RawCommand> = walk.group.drain(at..).collect();
                for m in &members {
                    below = below.max(walk.below.remove(&m.offset).unwrap_or(0));
                }
                let steps = members.len() - 1 + below;
                let r = if steps > MAX_CHAIN {
                    Err(TargetProblem::Writes)
                } else {
                    Ok(steps)
                };
                let mut memo = self.memo.borrow_mut();
                for m in &members {
                    memo.insert(m.offset, r);
                    walk.index.remove(&m.offset);
                }
                drop(memo);
                let steps = r?;
                match walk.frames.last_mut() {
                    Some(parent) => parent.below = parent.below.max(steps + 1),
                    None => return Ok(steps),
                }
            } else {
                // Not the first command of its group: keep its steps for
                // the group, and give its low index to the caller.
                walk.below.insert(frame.cmd.offset, below);
                if let Some(parent) = walk.frames.last_mut() {
                    parent.low = parent.low.min(frame.low);
                }
            }
        }
        unreachable!("the walk returns when the first frame leaves the stack")
    }

    /// Puts `cmd` on the stacks of the walk, after the check of its own
    /// actions.
    fn enter(&self, cmd: &'a RawCommand, walk: &mut Walk<'a>) -> Result<(), TargetProblem> {
        let next = match self.own_problem(cmd) {
            Ok(next) => next,
            Err(p) => {
                self.memo.borrow_mut().insert(cmd.offset, Err(p));
                return Err(p);
            }
        };
        let at = walk.counter;
        walk.counter += 1;
        walk.index.insert(cmd.offset, at);
        walk.group.push(cmd);
        walk.frames.push(Frame {
            cmd,
            next,
            k: 0,
            at,
            low: at,
            below: 0,
        });
        Ok(())
    }

    /// The code hash of a type 71 action is in the reviewed table.
    fn reviewed_code(&self, a: &RawAction) -> bool {
        let Some(hash) = a.code_hash else {
            return false;
        };
        let hex: String = hash.iter().map(|b| format!("{b:02x}")).collect();
        self.reviewed.contains(&hex.as_str())
    }
}

/// One command on the stack of the target walk.
struct Frame<'a> {
    cmd: &'a RawCommand,
    /// The commands that it executes.
    next: Vec<&'a RawCommand>,
    /// The next of them to walk.
    k: usize,
    /// The walk order of the command.
    at: usize,
    /// The lowest walk order of a command of its group that it reaches.
    low: usize,
    /// The most steps below it through the commands of other groups.
    below: usize,
}

/// The state of one target walk (see [`Index::height`]).
#[derive(Default)]
struct Walk<'a> {
    frames: Vec<Frame<'a>>,
    /// The commands of the groups that are not complete yet.
    group: Vec<&'a RawCommand>,
    /// The walk order of each command in `group`, by its offset.
    index: HashMap<usize, usize>,
    /// The steps below each command in `group` that left the stack.
    below: HashMap<usize, usize>,
    counter: usize,
}

// ---------------------------------------------------------------------
// The names that a phrase gives (execute by name)
// ---------------------------------------------------------------------

/// The normal form of a text, as words: lowercase, and each character
/// that is not a letter or a digit is a space. Two names with the same
/// normal form count as one name.
fn words_of(text: &str) -> Vec<String> {
    text.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

/// One part of a phrase.
enum Part {
    /// Words that must come in this order.
    Words(Vec<String>),
    /// A group: one of the alternatives. An alternative can be empty.
    Group(Vec<Vec<Part>>),
    /// A range: one word of digits.
    Number,
    /// A variable: any words, also none.
    Any,
}

/// The parts of a text, up to the end or to the `]` of the group. `depth`:
/// the number of open groups (0 outside a group). `None` for a text that
/// the builder cannot read: an unclosed group or variable, a `]` without a
/// group, or a group deeper than `MAX_GROUP_DEPTH`. The parser counts the
/// depth itself, so the limit holds for its own reading of the text.
fn parse_parts(text: &str, depth: usize) -> Option<(Vec<Vec<Part>>, &str)> {
    let in_group = depth > 0;
    let mut alternatives = Vec::new();
    let mut parts = Vec::new();
    let mut words = String::new();
    let mut rest = text;
    // A range is a whole alternative of a group.
    let flush = |words: &mut String, parts: &mut Vec<Part>| {
        if in_group && parts.is_empty() && crate::number_phrase::is_range(words) {
            parts.push(Part::Number);
        } else {
            let w = words_of(words);
            if !w.is_empty() {
                parts.push(Part::Words(w));
            }
        }
        words.clear();
    };
    loop {
        let Some(c) = rest.chars().next() else {
            if in_group {
                return None;
            }
            flush(&mut words, &mut parts);
            alternatives.push(parts);
            return Some((alternatives, rest));
        };
        rest = &rest[c.len_utf8()..];
        match c {
            '[' => {
                flush(&mut words, &mut parts);
                if depth >= MAX_GROUP_DEPTH {
                    return None;
                }
                let (alts, after) = parse_parts(rest, depth + 1)?;
                parts.push(Part::Group(alts));
                rest = after;
            }
            ']' if in_group => {
                flush(&mut words, &mut parts);
                alternatives.push(parts);
                return Some((alternatives, rest));
            }
            ']' | '}' => return None,
            '{' => {
                flush(&mut words, &mut parts);
                let close = rest.find('}')?;
                parts.push(Part::Any);
                rest = &rest[close + 1..];
            }
            ';' => {
                flush(&mut words, &mut parts);
                alternatives.push(std::mem::take(&mut parts));
            }
            _ => words.push(c),
        }
    }
}

/// The word positions after `parts`, from the positions of `from`.
/// `from[p]` is true when the parts before can end at word `p`. Each part
/// works on the whole set of positions one time, and each alternative of a
/// group too. Thus the work is at most the size of the phrase times the
/// square of the name length, with no exponential growth for groups in
/// groups.
fn advance(parts: &[Part], from: Vec<bool>, name: &[String]) -> Vec<bool> {
    let n = name.len();
    let mut now = from;
    for part in parts {
        let mut next = vec![false; n + 1];
        match part {
            Part::Words(w) => {
                for p in (0..=n).filter(|&p| now[p]) {
                    if name.get(p..p + w.len()) == Some(w.as_slice()) {
                        next[p + w.len()] = true;
                    }
                }
            }
            Part::Number => {
                for p in (0..n).filter(|&p| now[p]) {
                    let w = &name[p];
                    if !w.is_empty() && w.bytes().all(|b| b.is_ascii_digit()) {
                        next[p + 1] = true;
                    }
                }
            }
            Part::Any => {
                // Each position at or after a reached position.
                let mut seen = false;
                for p in 0..=n {
                    seen |= now[p];
                    next[p] = seen;
                }
            }
            Part::Group(alternatives) => {
                for alt in alternatives {
                    let reached = advance(alt, now.clone(), name);
                    for (slot, r) in next.iter_mut().zip(reached) {
                        *slot |= r;
                    }
                }
            }
        }
        if !next.contains(&true) {
            return next;
        }
        now = next;
    }
    now
}

/// The phrase can give the name (lowercase words) as VoiceAttack expands
/// it: one alternative of each group, a number of each range, any text
/// for a variable, and each phrase of a list with `;`. The check is
/// generous: true also for a phrase that the builder cannot read, and for
/// a phrase with groups deeper than `MAX_GROUP_DEPTH`.
#[cfg(test)]
fn phrase_can_give(phrase: &str, name: &[String]) -> bool {
    match parse_parts(phrase, 0) {
        Some((phrases, _)) => parts_can_give(&phrases, name),
        None => true,
    }
}

/// [`phrase_can_give`] on the parts of a phrase that `parse_parts` read.
fn parts_can_give(phrases: &[Vec<Part>], name: &[String]) -> bool {
    let mut start = vec![false; name.len() + 1];
    start[0] = true;
    phrases
        .iter()
        .any(|parts| advance(parts, start.clone(), name)[name.len()])
}

/// The 16 bytes of a GUID text in the .NET byte order (the order of the
/// command id in field 0).
fn parse_guid(text: &str) -> Option<[u8; 16]> {
    let t = text.as_bytes();
    if t.len() != 36 || [8, 13, 18, 23].iter().any(|&k| t[k] != b'-') {
        return None;
    }
    let hex: Vec<u8> = t.iter().copied().filter(|&b| b != b'-').collect();
    if hex.len() != 32 || !hex.iter().all(u8::is_ascii_hexdigit) {
        return None;
    }
    let mut v = [0u8; 16];
    for (k, pair) in hex.chunks(2).enumerate() {
        let s = std::str::from_utf8(pair).ok()?;
        v[k] = u8::from_str_radix(s, 16).ok()?;
    }
    v[0..4].reverse();
    v[4..6].reverse();
    v[6..8].reverse();
    Some(v)
}

// ---------------------------------------------------------------------
// The system-slot parse (whitelist item 6)
// ---------------------------------------------------------------------

const SLOT_SHAPE: [i32; 10] = [
    SET_TEXT, COND_BEGIN, SET_INT, SET_INT, SET_INT, SET_INT, SET_TEXT, COND_ELSE, SET_TEXT,
    COND_END,
];
const SLOT_FOLDER_VAR: &str = "DynamicVarLib";
const SLOT_FILE_PREFIX: &str = r"{TXT:DynamicVarLib}\EDCoPilotSystem";
const SLOT_VAR: &str = "SystemFromFile";
const SLOT_START: &str = "Startpos";
const SLOT_LENGTH: &str = "SysLength";
const SLOT_FALLBACK: &str = "SystemName";
/// The variables of the system-slot parse.
const SLOT_VARS: [&str; 5] = [
    SLOT_FOLDER_VAR,
    SLOT_VAR,
    SLOT_START,
    SLOT_LENGTH,
    SLOT_FALLBACK,
];

/// The start and the file number of the system-slot parse before the
/// write `wk`: one fixed sequence of ten actions at depth 0 that reads the
/// slot file and takes its first entry into `SystemName`. Each field of
/// the sequence must have the value of the audit, and each action must
/// have the tested settings.
fn slot_shape(actions: &[RawAction], wk: usize, depth: &[i32]) -> Option<(usize, u32)> {
    (0..wk.saturating_sub(9)).find_map(|st| {
        let s = &actions[st..st + 10];
        let kinds: Vec<i32> = s.iter().map(|a| a.kind).collect();
        if kinds != SLOT_SHAPE
            || depth[st] != 0
            || s.iter().any(|a| a.flag28 || a.unknown_setting.is_some())
        {
            return None;
        }
        let file = text(&s[0].text8)?
            .strip_prefix(SLOT_FILE_PREFIX)?
            .strip_suffix(".txt")?;
        if file.is_empty() || !file.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let number: u32 = file.parse().ok()?;
        let read = text(&s[0].text5) == Some(SLOT_VAR)
            && s[0].int13 == 4
            && s[0].int12 == 0
            && text(&s[0].text7).is_none()
            && text(&s[0].text14).is_none()
            && text(&s[0].text15).is_none();
        let check = text(&s[1].variable) == Some("{TXT:SystemFromFile}")
            && s[1].operator == OP_CONTAINS
            && text(&s[1].text6) == Some(";")
            && simple_compare(&s[1]);
        let names = [&s[2], &s[3], &s[4], &s[5]].map(|a| text(&a.text14));
        // The unread fields of the four set-integer actions, as each slot
        // parse of the tested profile has them.
        let fields = [&s[2], &s[3], &s[4], &s[5]].map(|a| (a.int10, a.int11, a.int19));
        let ints = [s[2].int13, s[3].int13, s[4].int13, s[5].int13] == [6, 8, 6, 9]
            && names
                == [
                    Some(SLOT_START),
                    Some(SLOT_START),
                    Some(SLOT_LENGTH),
                    Some(SLOT_LENGTH),
                ]
            && fields == [(0, 0, 0), (1, 1, 0), (0, 0, 0), (0, 1, 1)]
            && text(&s[3].text15).is_none()
            && text(&s[2].text15) == Some(r#"{TXTPOS:"[":SystemFromFile:0}"#)
            && text(&s[4].text15) == Some(r#"{TXTPOS:";":SystemFromFile:0}"#)
            && text(&s[5].text15) == Some("{INT:Startpos}");
        let set = |a: &RawAction, value: &str| {
            plain(a) && text(&a.text5) == Some(SLOT_FALLBACK) && text(&a.text6) == Some(value)
        };
        let sets = set(&s[6], "{TXTSUBSTR:SystemFromFile:Startpos:SysLength}")
            && set(&s[8], "{TXT:SystemFromFile}");
        (read && check && ints && sets).then_some((st, number))
    })
}

/// An action that sets a variable of the system-slot parse: a set-text
/// (field 5) or a set-integer (field 14).
fn sets_slot_var(a: &RawAction) -> bool {
    let name = match a.kind {
        SET_TEXT => text(&a.text5),
        SET_INT => text(&a.text14),
        _ => None,
    };
    name.is_some_and(|n| eq_any(SLOT_VARS, n))
}

/// The number `n` of each system slot `{TXT:System<n>}` of the phrase.
fn system_slots(phrase: &str) -> Vec<u32> {
    let mut slots: Vec<u32> = txt_vars(phrase)
        .into_iter()
        .filter_map(|v| v.strip_prefix("System"))
        .filter(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
        .filter_map(|d| d.parse().ok())
        .collect();
    slots.sort_unstable();
    slots.dedup();
    slots
}

/// The phrase has one system slot, `System<n>`, and `n` is the number of
/// the slot file. (The matcher fills the fallback only when it captures
/// one slot. Another captured slot gives no token, thus a miss.)
fn check_slot_number(phrase: &str, file: u32) -> Result<(), String> {
    let number = match system_slots(phrase).as_slice() {
        [one] => Some(*one),
        _ => None,
    };
    match number {
        Some(n) if n == file => Ok(()),
        _ => Err(format!(
            "the command reads the slot file {file}, but its phrase slot has another number"
        )),
    }
}

// ---------------------------------------------------------------------
// The suffix shape (whitelist item 7)
// ---------------------------------------------------------------------

/// The prefix rules of a suffix and the actions of its known shape.
pub(super) struct SuffixShape {
    pub rules: Vec<PrefixRule>,
    /// The indices of the actions of the shape.
    pub region: Vec<usize>,
}

/// The known suffix shape: first `{PREFIX}` contains blocks with one plain
/// set-text each (a text compare, no second variable, no compound
/// condition, the tested field 12). Then one "has not been set" block on a
/// rule variable that is in the token, with inert actions, an else branch
/// with only the write, and the end. The end is the last action. Actions
/// before the blocks follow the rules before the write.
///
/// `wk`: the index of the request write.
pub(super) fn suffix_shape(
    actions: &[RawAction],
    token: &str,
    wk: usize,
) -> Result<SuffixShape, String> {
    let clip = token.contains("{CLIP}");
    let token_vars = txt_vars(token);
    let mut rules: Vec<PrefixRule> = Vec::new();
    let mut region = Vec::new();
    let mut closed = false;
    let mut k = 0;
    while k < wk {
        let a = &actions[k];
        if a.kind == COND_BEGIN && text(&a.variable) == Some("{PREFIX}") {
            let block = actions.get(k + 1..k + 3);
            let rule = match block {
                Some([set, end])
                    if a.operator == OP_CONTAINS
                        && simple_compare(a)
                        && a.int12 == 1
                        && a.field2_zero
                        && plain(set)
                        && end.kind == COND_END =>
                {
                    match (text(&a.text6), text(&set.text5), text(&set.text6)) {
                        (Some(contains), Some(name), Some(value)) if !value.contains('{') => {
                            Some(PrefixRule {
                                contains: contains.to_string(),
                                set: HashMap::from([(name.to_string(), value.to_string())]),
                            })
                        }
                        _ => None,
                    }
                }
                _ => None,
            };
            let Some(rule) = rule else {
                return Err("the suffix has a {PREFIX} block of an unknown shape".into());
            };
            rules.push(rule);
            region.extend(k..k + 3);
            k += 3;
            continue;
        }
        if a.kind == COND_BEGIN {
            let unknown = || "the suffix has a condition block of an unknown shape".to_string();
            // The tested block: no compare value and field 2 = 0.0.
            let tested = a.operator == OP_NOT_SET
                && simple_compare(a)
                && a.int12 == 0
                && a.field2_zero
                && text(&a.text6).is_none();
            if !tested {
                return Err(unknown());
            }
            // The checked variable: a variable of the rules, in the token.
            let checked = text(&a.variable).unwrap_or("");
            let of_rules = rules.iter().any(|r| eq_any(r.set.keys(), checked));
            if !of_rules || !eq_any(&token_vars, checked) {
                return Err(unknown());
            }
            let el = actions[k + 1..wk]
                .iter()
                .position(|x| x.kind == COND_ELSE)
                .map(|p| k + 1 + p)
                .ok_or_else(unknown)?;
            let known = actions[k + 1..el].iter().all(|x| is_inert(x.kind, clip))
                && el + 1 == wk
                && wk + 2 == actions.len()
                && actions[wk + 1].kind == COND_END;
            if !known {
                return Err(unknown());
            }
            region.extend(k..=wk + 1);
            closed = true;
            break;
        }
        k += 1;
    }
    if !closed {
        return Err("the suffix has no \"has not been set\" block around the write".into());
    }
    if rules.is_empty() {
        return Err("the suffix has no prefix rules".into());
    }
    if case_conflict(rules.iter().flat_map(|r| r.set.keys().map(String::as_str))) {
        return Err("the suffix sets variables whose names differ only in case".into());
    }
    Ok(SuffixShape { rules, region })
}

// ---------------------------------------------------------------------
// The checks of one command
// ---------------------------------------------------------------------

fn eq_any(names: impl IntoIterator<Item = impl AsRef<str>>, var: &str) -> bool {
    names
        .into_iter()
        .any(|n| n.as_ref().eq_ignore_ascii_case(var))
}

/// The set-text actions of `var` (names without regard to case).
fn sets_of_var<'a>(actions: &'a [RawAction], var: &str) -> Vec<(usize, &'a RawAction)> {
    actions
        .iter()
        .enumerate()
        .filter(|(_, a)| {
            a.kind == SET_TEXT && text(&a.text5).is_some_and(|n| n.eq_ignore_ascii_case(var))
        })
        .collect()
}

/// Assumption 3: a number variable has exactly one plain set-text from
/// `{TXTNUM:LastCMD}` at depth 0 before the write. Each set of `LastCMD`
/// is a plain `{CMD}` at depth 0, before it. The record holds no value of
/// these actions: the matcher takes the number from the spoken text.
fn check_number_vars(
    actions: &[RawAction],
    wk: usize,
    rec: &Command,
    depth: &[i32],
) -> Result<(), String> {
    for v in &rec.number_vars {
        let sets = sets_of_var(&actions[..wk], v);
        let last = sets_of_var(&actions[..wk], "LastCMD");
        let good = match sets.as_slice() {
            [(k, a)] => {
                plain(a)
                    && depth[*k] == 0
                    && text(&a.text6) == Some(NUM_OF_CMD)
                    && !last.is_empty()
                    && last.iter().all(|(j, l)| {
                        j < k && plain(l) && depth[*j] == 0 && text(&l.text6) == Some("{CMD}")
                    })
            }
            _ => false,
        };
        if !good {
            return Err("the number variable is not set as the program models it".into());
        }
    }
    Ok(())
}

/// The variables that the builder can take as a constant of the command.
const CONST_VARS: [&str; 1] = ["SystemName"];

/// Assumption 4: the constants of the token, each with the index of the
/// action that gives its value. A variable of [`CONST_VARS`] in the token
/// is a constant when exactly one set-text before the write sets it (names
/// without regard to case), outside the actions in `skip` (the system-slot
/// parse and the suffix shape). That set must be plain, at depth 0, with a
/// value that is not empty and has no `{`. The value comes from that
/// action, and the walk of [`check_record`] accepts only that action as a
/// set of the variable. Thus each other shape of the sets before the write
/// leaves the command out. A set after the write does not change the
/// token.
fn constants_of(
    actions: &[RawAction],
    wk: usize,
    token: &str,
    depth: &[i32],
    skip: &[bool],
) -> Vec<(String, String, usize)> {
    let mut out: Vec<(String, String, usize)> = Vec::new();
    for var in txt_vars(token) {
        if !CONST_VARS.contains(&var) || out.iter().any(|(v, _, _)| v == var) {
            continue;
        }
        let sets: Vec<(usize, &RawAction)> = sets_of_var(&actions[..wk], var)
            .into_iter()
            .filter(|(k, _)| !skip[*k])
            .collect();
        if let [(k, a)] = sets.as_slice() {
            let value = text(&a.text6).filter(|v| !v.contains('{'));
            if let (true, Some(value)) = (plain(a) && depth[*k] == 0, value) {
                out.push((var.to_string(), value.to_string(), *k));
            }
        }
    }
    out
}

/// Assumption 6: when the token takes `SystemName` or `spokenQueuedName`
/// from the spoken slot (the fallback of the matcher), the command must
/// read that slot with the system-slot parse.
fn check_fallback(phrase: &str, rec: &Command, slot_parse: bool) -> Result<(), String> {
    let phrase_slots = txt_vars(phrase);
    for v in txt_vars(&rec.token_template) {
        if v != SLOT_FALLBACK && v != "spokenQueuedName" {
            continue;
        }
        let composite = rec.prefix_rules.iter().any(|r| eq_any(r.set.keys(), v));
        if eq_any(rec.constants.keys(), v)
            || eq_any(&rec.number_vars, v)
            || phrase_slots.contains(&v)
            || composite
            || phrase_slots.is_empty()
        {
            continue;
        }
        if v == SLOT_FALLBACK && slot_parse {
            continue;
        }
        return Err(
            "the token takes the spoken slot, but the command does not read the slot file".into(),
        );
    }
    Ok(())
}

/// A token placeholder that the matcher fills from a phrase slot of the
/// same name gets the first name of the slot. VoiceAttack writes the whole
/// text of the slot variable. The two are the same only when the form
/// check of the slot files (`crate::dynamic_vocab`) requires one plain name
/// in the file of the slot kind. Thus each other phrase slot in the token
/// leaves the command out: a system slot (its file holds a list), and a
/// variable of no slot kind. A constant, a number variable and a variable
/// of the prefix rules do not come from the slot.
fn check_slot_tokens(phrase: &str, rec: &Command) -> Result<(), String> {
    let phrase_vars = txt_vars(phrase);
    let rule_vars = rec.prefix_rules.iter().flat_map(|r| r.set.keys());
    let rule_vars: Vec<&String> = rule_vars.collect();
    for v in txt_vars(&rec.token_template) {
        let from_slot = phrase_vars.contains(&v)
            && !rec.constants.contains_key(v)
            && !rec.number_vars.iter().any(|n| n == v)
            && !eq_any(&rule_vars, v);
        let one_name = slot_kind(v).is_some_and(holds_one_plain_name);
        if from_slot && !one_name {
            return Err(SLOT_TEXT.into());
        }
    }
    Ok(())
}

/// The whitelist for a full or suffix command with one request write.
/// It also sets the constants of the record (see [`constants_of`]).
/// `wk`: the index of the request write. `suffix_region`: the actions of
/// the suffix shape (see [`suffix_shape`]), empty for a full command.
pub(super) fn check_record(
    c: &RawCommand,
    rec: &mut Command,
    index: &Index,
    wk: usize,
    suffix_region: &[usize],
) -> Result<(), String> {
    let actions = &c.actions;
    if actions[..=wk].iter().any(|a| a.flag28) {
        return Err(SWITCHED_OFF.into());
    }
    // At any depth, also in the suffix shape and in the slot parse.
    if actions[..=wk].iter().any(|a| a.token_name) {
        return Err(TOKEN_NAME.into());
    }
    check_operators(actions)?;
    let depth = depths(actions);
    let slot = slot_shape(actions, wk, &depth);
    let in_slot = |k: usize| slot.is_some_and(|(start, _)| (start..start + 10).contains(&k));
    let changed_setting = actions.iter().enumerate().any(|(k, a)| {
        unknown_setting(a) || (a.kind == SET_INT && in_slot(k) && a.unknown_setting.is_some())
    });
    if changed_setting {
        return Err(UNKNOWN_SETTING.into());
    }
    let path_vars = write_path_vars();
    if actions[..wk].iter().any(|a| sets_write_path(a, &path_vars)) {
        return Err(WRITE_PATH.into());
    }
    let phrase = text(&c.phrase).unwrap_or("");
    // The number check of the phrase runs in `extract`, with the grammar of
    // the profile variables in the phrase.
    // VoiceAttack makes the number of a composite command from the prefix
    // text too.
    if !rec.number_vars.is_empty() && rec.command_type == CommandType::Suffix {
        return Err(NUMBER_SUFFIX.into());
    }
    let token = rec.token_template.as_str();
    let clip = token.contains("{CLIP}");
    let token_vars: HashSet<String> = placeholders(token)
        .into_iter()
        .filter_map(|p| match p {
            Place::Txt(v) | Place::Num(v) => Some(v.to_lowercase()),
            _ => None,
        })
        .collect();
    let mut skip = vec![false; actions.len()];
    if let Some((start, file)) = slot {
        check_slot_number(phrase, file)?;
        let other_set = actions[..wk]
            .iter()
            .enumerate()
            .any(|(k, a)| !in_slot(k) && sets_slot_var(a));
        if other_set {
            return Err(SLOT_VARIABLE.into());
        }
        skip[start..start + 10].fill(true);
    }
    // Fix round 10: a set-text before the write outside the suffix shape
    // and outside the slot parse must not set a variable of the prefix
    // rules. Before the blocks, VoiceAttack writes the value of the block,
    // but the constant would get the value of the plain set. After the
    // blocks, the program does not model the order. Thus each order leaves
    // the suffix out.
    let rule_set_outside = actions[..wk].iter().enumerate().any(|(k, a)| {
        a.kind == SET_TEXT
            && !skip[k]
            && !suffix_region.contains(&k)
            && text(&a.text5)
                .is_some_and(|n| rec.prefix_rules.iter().any(|r| eq_any(r.set.keys(), n)))
    });
    if rule_set_outside {
        return Err(RULE_VAR_SET.into());
    }
    for &k in suffix_region {
        skip[k] = true;
    }
    let constants = constants_of(actions, wk, token, &depth, &skip);
    let constant_set = |k: usize| constants.iter().any(|(_, _, at)| *at == k);
    let number_var = |name: &str| eq_any(&rec.number_vars, name);
    let mut d = 0;
    // The `else` actions of the open block before the write. A block has at
    // most one: a second has no meaning that the program knows.
    let mut elses = 0;
    for (k, a) in actions.iter().enumerate() {
        if skip[k] {
            continue;
        }
        if k == wk {
            // A full command with the write in a block is in the class
            // `maintenance`. The write of a suffix is in its shape.
            continue;
        }
        if k > wk {
            match a.kind {
                kind if is_inert(kind, false)
                    || SIDE_EFFECT.contains(&kind)
                    || AFTER_WRITE_EXTRA.contains(&kind) => {}
                EXECUTE => {
                    let target = index.resolve(a).ok_or_else(|| TARGET_UNKNOWN.to_string())?;
                    match index.check_target(target) {
                        Ok(()) => {}
                        Err(TargetProblem::Unknown) => return Err(TARGET_UNKNOWN.into()),
                        Err(TargetProblem::Writes) => return Err(TARGET_WRITES.into()),
                        Err(TargetProblem::Code) => return Err(TARGET_CODE.into()),
                        Err(TargetProblem::Setting) => return Err(TARGET_SETTING.into()),
                    }
                }
                kind => return Err(not_allowed(kind, "after the write")),
            }
            continue;
        }
        // Before the write: whitelist items 1 to 5.
        match a.kind {
            COND_BEGIN if d == 0 => {
                d += 1;
                elses = 0;
            }
            COND_ELSE if d > 0 && elses == 0 => elses += 1,
            COND_END if d > 0 => d -= 1,
            COND_BEGIN | COND_ELSE | COND_END => {
                return Err("a condition block before the write has an unknown shape".into())
            }
            kind if is_inert(kind, clip) || OTHER_VARIABLE.contains(&kind) => {}
            SET_TEXT => {
                let name = lower(&a.text5);
                let modelled = constant_set(k) || (plain(a) && d == 0 && number_var(&name));
                if token_vars.contains(&name) && !modelled {
                    return Err(TOKEN_VARIABLE.into());
                }
            }
            kind => return Err(not_allowed(kind, "before the write")),
        }
    }
    if d != 0 {
        return Err("a condition block before the write has an unknown shape".into());
    }
    check_number_vars(actions, wk, rec, &depth)?;
    rec.constants = constants
        .into_iter()
        .map(|(var, value, _)| (var, value))
        .collect();
    check_slot_tokens(phrase, rec)?;
    check_fallback(phrase, rec, slot.is_some())
}

/// The whitelist for a prefix: no action that is switched off, the types
/// of the items 1 and 2, condition blocks without a set, and plain
/// set-text actions at depth 0 with a literal value. The program uses each
/// set as it is.
pub(super) fn check_prefix(c: &RawCommand) -> Result<(), String> {
    if c.actions.iter().any(|a| a.flag28) {
        return Err(PREFIX_SWITCHED_OFF.into());
    }
    if c.actions.iter().any(|a| a.token_name) {
        return Err(TOKEN_NAME.into());
    }
    if c.actions.iter().any(unknown_setting) {
        return Err(UNKNOWN_SETTING.into());
    }
    let path_vars = write_path_vars();
    if c.actions.iter().any(|a| sets_write_path(a, &path_vars)) {
        return Err(WRITE_PATH.into());
    }
    check_operators(&c.actions)?;
    let mut d = 0;
    for a in &c.actions {
        match a.kind {
            COND_BEGIN if d == 0 => d += 1,
            COND_ELSE if d > 0 => {}
            COND_END if d > 0 => d -= 1,
            COND_BEGIN | COND_ELSE | COND_END => {
                return Err("a condition block of the prefix has an unknown shape".into())
            }
            kind if is_inert(kind, false) || OTHER_VARIABLE.contains(&kind) => {}
            SET_TEXT => {
                let literal =
                    text(&a.text5).is_some() && text(&a.text6).is_some_and(|v| !v.contains('{'));
                if d != 0 || !plain(a) || !literal {
                    return Err(PREFIX_SET.into());
                }
            }
            kind => return Err(not_allowed(kind, "in a prefix")),
        }
    }
    if d != 0 {
        return Err("a condition block of the prefix has an unknown shape".into());
    }
    let names = c
        .actions
        .iter()
        .filter(|a| a.kind == SET_TEXT)
        .filter_map(|a| text(&a.text5));
    if case_conflict(names) {
        return Err("the prefix sets variables whose names differ only in case".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    #[test]
    fn test_parse_guid_uses_the_dotnet_byte_order() {
        let id = parse_guid("04030201-0605-0807-090a-0b0c0d0e0f10").unwrap();
        assert_eq!(id, core::array::from_fn(|k| k as u8 + 1));
        assert_eq!(parse_guid("not a guid"), None);
        assert_eq!(parse_guid("04030201x0605-0807-090a-0b0c0d0e0f10"), None);
        assert_eq!(parse_guid("0403020g-0605-0807-090a-0b0c0d0e0f10"), None);
    }

    #[test]
    fn test_type_names() {
        assert_eq!(type_name(71), "inline code");
        assert_eq!(type_name(99), "not known");
        assert!(SET_INT == 37 && in_table(SET_INT));
    }

    #[test]
    fn test_write_path_vars_come_from_the_request_path() {
        assert_eq!(write_path_vars(), ["pythonlibpath", "requestfile"]);
    }

    #[test]
    fn test_case_conflict() {
        assert!(case_conflict(["Size", "size"]));
        assert!(!case_conflict(["Size", "Size", "Colour"]));
    }

    /// An execute action by id.
    fn exec(id: [u8; 16]) -> RawAction {
        let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
        let rev = |b: &[u8]| hex(&b.iter().rev().copied().collect::<Vec<_>>());
        RawAction {
            kind: EXECUTE,
            text5: Some(format!(
                "{}-{}-{}-{}-{}",
                rev(&id[0..4]),
                rev(&id[4..6]),
                rev(&id[6..8]),
                hex(&id[8..10]),
                hex(&id[10..16])
            )),
            exec_by_name: Some(false),
            ..RawAction::default()
        }
    }

    fn id_of(n: usize) -> [u8; 16] {
        let mut id = [0u8; 16];
        id[..8].copy_from_slice(&(n as u64 + 1).to_le_bytes());
        id
    }

    /// `n` commands; command `k` executes command `k + 1`. The last one
    /// plays a sound.
    fn chain(n: usize, offset: usize) -> Vec<RawCommand> {
        (0..n)
            .map(|k| RawCommand {
                offset: offset + k,
                id: id_of(offset + k),
                phrase: Some(format!("link {}", offset + k)),
                category: None,
                command_type: 0,
                group: None,
                enabled: true,
                voice_trigger: true,
                unknown_setting: None,
                actions: vec![if k + 1 < n {
                    exec(id_of(offset + k + 1))
                } else {
                    RawAction {
                        kind: 14,
                        ..RawAction::default()
                    }
                }],
            })
            .collect()
    }

    fn words(text: &str) -> Vec<String> {
        words_of(text)
    }

    /// Nested groups with a variable at each level: the work stays small.
    #[test]
    fn test_name_check_of_nested_groups_finishes_quickly() {
        let mut phrase = "lighthouse".to_string();
        for _ in 0..40 {
            phrase = format!("[{{TXT:Keeper}} {phrase}; harbour]");
        }
        let mut name = words(&"quay ".repeat(19));
        name.push("buoy".to_string());
        assert_eq!(name.len(), 20);
        let start = Instant::now();
        assert!(!phrase_can_give(&phrase, &name));
        let mut good = words(&"quay ".repeat(19));
        good.push("lighthouse".to_string());
        assert!(phrase_can_give(&phrase, &good));
        assert!(start.elapsed().as_secs_f64() < 1.0, "{:?}", start.elapsed());
    }

    /// Groups deeper than the limit can give any name. The check does not
    /// overflow the stack.
    #[test]
    fn test_name_check_of_too_deep_groups_gives_any_name() {
        let nested = |n: usize| format!("{}harbour{}", "[".repeat(n), "]".repeat(n));
        let name = words("lighthouse");
        assert!(!phrase_can_give(&nested(MAX_GROUP_DEPTH), &name));
        assert!(phrase_can_give(&nested(MAX_GROUP_DEPTH + 1), &name));
        assert!(phrase_can_give(&nested(100_000), &name));
        assert!(phrase_can_give(&"[".repeat(100_000), &name));
        // Brackets in a placeholder are no group for the parser: a scan of
        // the raw text sees depth 1 here, the parser sees 100,000.
        let hidden = format!("{}harbour{}", "[{]}".repeat(100_000), "]".repeat(100_000));
        assert!(phrase_can_give(&hidden, &name));
        let hidden = format!("{}harbour{}", "[{x}".repeat(100_000), "]".repeat(100_000));
        assert!(phrase_can_give(&hidden, &name));
    }

    #[test]
    fn test_long_execute_chain_finishes_without_a_stack_overflow() {
        let cmds = chain(100_000, 0);
        let index = Index::new(&cmds, &[]);
        let start = Instant::now();
        // The chain is longer than the limit: it counts as "can write".
        assert_eq!(index.check_target(&cmds[0]), Err(TargetProblem::Writes));
        // A short end of the chain is fine.
        assert_eq!(index.check_target(&cmds[100_000 - 10]), Ok(()));
        assert!(start.elapsed().as_secs_f64() < 2.0, "{:?}", start.elapsed());
    }

    /// Commands that execute each other in a loop count one step for each
    /// command of the loop after the first. A short loop is fine, a loop
    /// longer than the chain limit can write.
    #[test]
    fn test_a_loop_of_execute_actions_counts_its_commands() {
        let ring = |n: usize, offset: usize| {
            let mut cmds = chain(n, offset);
            let last = cmds.last_mut().unwrap();
            last.actions = vec![exec(id_of(offset))];
            cmds
        };
        let small = ring(5, 0);
        let index = Index::new(&small, &[]);
        assert_eq!(index.height(&small[2]), Ok(4));
        assert_eq!(index.check_target(&small[0]), Ok(()));
        let large = ring(MAX_CHAIN + 2, 0);
        let index = Index::new(&large, &[]);
        assert_eq!(index.check_target(&large[0]), Err(TargetProblem::Writes));
        // A command that runs into the loop counts the loop below it.
        let mut cmds = ring(5, 0);
        cmds.extend(chain(1, 100));
        cmds[5].actions = vec![exec(id_of(3))];
        let index = Index::new(&cmds, &[]);
        assert_eq!(index.height(&cmds[5]), Ok(5));
    }

    #[test]
    fn test_many_callers_of_one_chain_finish_quickly() {
        let mut cmds = chain(MAX_CHAIN, 0);
        for k in 0..2000 {
            cmds.push(RawCommand {
                offset: 1_000_000 + k,
                id: id_of(1_000_000 + k),
                phrase: Some(format!("caller {k}")),
                category: None,
                command_type: 0,
                group: None,
                enabled: true,
                voice_trigger: true,
                unknown_setting: None,
                actions: vec![exec(id_of(0))],
            });
        }
        let index = Index::new(&cmds, &[]);
        let start = Instant::now();
        for caller in &cmds[MAX_CHAIN..] {
            let target = index.resolve(&caller.actions[0]).unwrap();
            assert_eq!(index.check_target(target), Ok(()));
        }
        assert!(start.elapsed().as_secs_f64() < 1.0, "{:?}", start.elapsed());
    }
}
