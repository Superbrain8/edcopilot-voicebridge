use std::collections::HashMap;
use std::fs;
use std::path::Path;

use crate::command_map::Command;
use crate::command_map::CommandType;
use crate::grammar::{parse_grammar, GrammarNode};
use crate::number_phrase::{number_text_outside_ranges, NUMBER_TEXT_REASON};
use crate::profile::{BlockingPhrase, LeftOutClass};

const VAR_PREFIX: &str = "{TXT:EDCP_";

/// The start of the slot name of a fixed name slot.
pub const FIXED_SLOT_PREFIX: &str = "Fixed:";

/// The most spoken forms that one fixed name gives.
const MAX_FIXED_NAMES: usize = 16;

/// The largest INI file of the runtime grammar that the program reads. The
/// tested file has 64,683 bytes. A larger file reads as a file that does
/// not read: the phrases keep their variables (a miss).
const MAX_INI_BYTES: usize = 4 * 1024 * 1024;

/// The variables of the runtime grammar (`EDCoPilot.VoiceCommands.ini`):
/// the variable key without `EDCP_` to its grammar. The load reads the
/// file one time and gives it to [`resolve_runtime_grammar_from`] and
/// [`resolve_blocking_from`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimeGrammar(HashMap<String, String>);

impl RuntimeGrammar {
    /// The grammar of the INI file at `path`, or `None` when the file does
    /// not read or has more than [`MAX_INI_BYTES`] bytes.
    pub fn read(path: &Path) -> Option<RuntimeGrammar> {
        read_ini(path).map(RuntimeGrammar)
    }
}

/// A command that `resolve_runtime_grammar` takes out of the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemovedCommand {
    /// The phrase before the resolution.
    pub phrase: String,
    pub token_template: String,
    /// A short reason for a person.
    pub reason: String,
}

/// The result of `resolve_runtime_grammar`.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedGrammar {
    /// The commands with the runtime grammar in their phrases, in order.
    pub commands: Vec<Command>,
    /// The commands that the check of the resolved phrase took out.
    pub removed: Vec<RemovedCommand>,
}

/// Replace `{TXT:EDCP_*}` variables in phrases with the grammar from `VoiceCommands.ini`.
/// Unknown variables and all other text stay unchanged. If the file cannot be read,
/// the phrases stay unchanged.
///
/// A command with a constant system (`SystemName`) whose phrase ends with a
/// name variable gets a fixed name slot in place of the variable, for
/// example `{TXT:Fixed:HALVARD:Vennick}`. The names of the slot are the spoken
/// forms of the name grammar ("Halvard Crane") and the system name ("Vennick").
/// Thus the matcher scores the name alone, and the system name is a second
/// way to say the command.
///
/// A number command (a command with a number variable) whose resolved
/// phrase fails `number_text_outside_ranges` leaves the list. It is in
/// `removed`, with the reason. The check also runs when the file cannot be
/// read.
pub fn resolve_runtime_grammar(
    commands: Vec<Command>,
    voice_commands_ini: &Path,
) -> ResolvedGrammar {
    resolve_runtime_grammar_with(commands, Some(voice_commands_ini), &HashMap::new())
}

/// [`resolve_runtime_grammar`] with the profile variables for the number
/// check. `voice_commands_ini`: `None` reads no file (the phrases stay
/// unchanged), and the number check still runs. The number check sees the
/// phrase with the runtime grammar and with the grammar of each profile
/// variable in `profile_vars`. The phrases of the result do not have the
/// profile variables: [`resolve_profile_vars`] puts them in.
pub fn resolve_runtime_grammar_with(
    commands: Vec<Command>,
    voice_commands_ini: Option<&Path>,
    profile_vars: &HashMap<String, String>,
) -> ResolvedGrammar {
    let grammar = voice_commands_ini.and_then(RuntimeGrammar::read);
    resolve_runtime_grammar_from(commands, grammar.as_ref(), profile_vars)
}

/// [`resolve_runtime_grammar_with`] with the grammar that
/// [`RuntimeGrammar::read`] read. `None`: the phrases stay unchanged, and
/// the number check still runs.
pub fn resolve_runtime_grammar_from(
    commands: Vec<Command>,
    grammar: Option<&RuntimeGrammar>,
    profile_vars: &HashMap<String, String>,
) -> ResolvedGrammar {
    let vars = grammar.map(|g| &g.0);
    let mut out = ResolvedGrammar {
        commands: Vec::with_capacity(commands.len()),
        removed: Vec::new(),
    };
    for mut command in commands {
        let original = command.phrase.clone();
        if let Some(vars) = vars {
            add_fixed_name_slot(&mut command, vars);
            if let Some(phrase) = &command.phrase {
                command.phrase = Some(substitute(phrase, vars));
            }
        }
        let resolved = with_profile_vars(command.phrase.as_deref().unwrap_or(""), profile_vars);
        if !command.number_vars.is_empty() && number_text_outside_ranges(&resolved) {
            out.removed.push(RemovedCommand {
                phrase: original.unwrap_or_default(),
                token_template: command.token_template,
                reason: NUMBER_TEXT_REASON.to_string(),
            });
            continue;
        }
        out.commands.push(command);
    }
    out
}

/// Puts a fixed name slot in place of the last name variable of a command
/// with a constant system. See `resolve_runtime_grammar`.
fn add_fixed_name_slot(command: &mut Command, vars: &HashMap<String, String>) {
    let Some(system) = command.constants.get("SystemName").cloned() else {
        return;
    };
    let Some(phrase) = command.phrase.as_deref() else {
        return;
    };
    let trimmed = phrase.trim_end();
    let Some(start) = trimmed.rfind(VAR_PREFIX) else {
        return;
    };
    let Some(name) = trimmed[start..]
        .strip_prefix(VAR_PREFIX)
        .and_then(|rest| rest.strip_suffix('}'))
    else {
        return;
    };
    if name.is_empty() || name.contains(['{', '}', '[', ']', ';']) {
        return;
    }
    if system.contains(['{', '}', '[', ']', ';', '<', '>']) {
        return;
    }
    let mut names = match vars.get(name) {
        Some(trigger) => {
            let forms = spoken_forms(&parse_grammar(&as_group(trigger)));
            // Too many forms, or a slot or a range in the name: keep the
            // phrase as it is, so that the command does not lose its name.
            if forms.is_empty() {
                return;
            }
            forms
        }
        None => Vec::new(),
    };
    if !names.iter().any(|n| n.eq_ignore_ascii_case(&system)) {
        names.push(system.clone());
    }
    // The key has the system: two commands with the same name variable and
    // different systems keep their own names.
    let slot = format!("{FIXED_SLOT_PREFIX}{name}:{system}");
    command.phrase = Some(format!("{}{{TXT:{slot}}}", &trimmed[..start]));
    command.fixed_vocab.insert(slot, names);
}

/// All spoken forms of a grammar without slots, at most `MAX_FIXED_NAMES`.
/// A grammar with a slot or a range gives no form.
fn spoken_forms(node: &GrammarNode) -> Vec<String> {
    fn forms(node: &GrammarNode) -> Option<Vec<Vec<String>>> {
        Some(match node {
            GrammarNode::Literal(t) => vec![t.split_whitespace().map(str::to_string).collect()],
            GrammarNode::Alternation(v) => {
                let mut out = Vec::new();
                for n in v {
                    out.extend(forms(n)?);
                }
                out
            }
            GrammarNode::Optional(inner) => {
                let mut out = vec![Vec::new()];
                out.extend(forms(inner)?);
                out
            }
            GrammarNode::Sequence(v) => {
                let mut acc: Vec<Vec<String>> = vec![Vec::new()];
                for n in v {
                    let parts = forms(n)?;
                    let mut next = Vec::new();
                    for a in &acc {
                        for p in &parts {
                            let mut x = a.clone();
                            x.extend(p.iter().cloned());
                            next.push(x);
                        }
                    }
                    if next.len() > MAX_FIXED_NAMES {
                        return None;
                    }
                    acc = next;
                }
                acc
            }
            GrammarNode::ListSlot(_)
            | GrammarNode::Range(..)
            | GrammarNode::ScaledRange(..)
            | GrammarNode::Never => return None,
        })
    }
    let mut out: Vec<String> = Vec::new();
    for words in forms(node).unwrap_or_default() {
        let text = words.join(" ");
        if !text.is_empty() && !out.contains(&text) {
            out.push(text);
        }
    }
    out.truncate(MAX_FIXED_NAMES);
    out
}

/// The result of `resolve_blocking`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedBlocking {
    /// The blocking phrases with the runtime grammar and the grammar of the
    /// profile variables in them, in order.
    pub phrases: Vec<BlockingPhrase>,
    /// The phrases that still have a `{TXT:EDCP_` variable (the INI file
    /// does not give it, or the file does not read), or that do not parse
    /// (a group deeper than the parser reads). They are not in `phrases`.
    pub skipped: usize,
}

/// The blocking phrases of the commands that the number check took out
/// (`ResolvedGrammar::removed`). Each was a record: an enabled command
/// with a voice trigger. They are full commands: the builder leaves out
/// each suffix with a number variable. An internal phrase (`((...))`)
/// gives none, as in the builder.
pub fn removed_as_blocking(removed: &[RemovedCommand]) -> Vec<BlockingPhrase> {
    removed
        .iter()
        .filter(|r| {
            let p = r.phrase.trim();
            !p.is_empty() && !(p.starts_with("((") && p.ends_with("))"))
        })
        .map(|r| BlockingPhrase {
            phrase: r.phrase.clone(),
            class: LeftOutClass::NotUnderstood,
            command_type: CommandType::Full,
            composite_group: None,
        })
        .collect()
}

/// Puts the runtime grammar of `voice_commands_ini` and the grammar of
/// the profile variables into the blocking phrases, as the load does for
/// the commands. A phrase that does not resolve or does not parse is
/// skipped and counted.
pub fn resolve_blocking(
    phrases: Vec<BlockingPhrase>,
    voice_commands_ini: Option<&Path>,
    profile_vars: &HashMap<String, String>,
) -> ResolvedBlocking {
    let grammar = voice_commands_ini.and_then(RuntimeGrammar::read);
    resolve_blocking_from(phrases, grammar.as_ref(), profile_vars)
}

/// [`resolve_blocking`] with the grammar that [`RuntimeGrammar::read`]
/// read.
pub fn resolve_blocking_from(
    phrases: Vec<BlockingPhrase>,
    grammar: Option<&RuntimeGrammar>,
    profile_vars: &HashMap<String, String>,
) -> ResolvedBlocking {
    let vars = grammar.map(|g| &g.0);
    let mut out = ResolvedBlocking::default();
    for mut blocking in phrases {
        let phrase = match vars {
            Some(vars) => substitute(&blocking.phrase, vars),
            None => blocking.phrase.clone(),
        };
        let phrase = with_profile_vars(&phrase, profile_vars);
        if phrase.contains(VAR_PREFIX) || parse_grammar(&phrase) == GrammarNode::Never {
            out.skipped += 1;
            continue;
        }
        blocking.phrase = phrase;
        out.phrases.push(blocking);
    }
    out
}

/// Replace profile-level grammar variables (`{TXT:name}`) in phrases with
/// their grammar. All other text stays unchanged.
pub fn resolve_profile_vars(
    mut commands: Vec<Command>,
    vars: &HashMap<String, String>,
) -> Vec<Command> {
    for command in &mut commands {
        if let Some(phrase) = &mut command.phrase {
            *phrase = with_profile_vars(phrase, vars);
        }
    }
    commands
}

/// The phrase with the grammar of each profile variable of `vars` in
/// place of its `{TXT:name}`.
fn with_profile_vars(phrase: &str, vars: &HashMap<String, String>) -> String {
    let mut phrase = phrase.to_string();
    for (name, grammar) in vars {
        let variable = format!("{{TXT:{name}}}");
        if phrase.contains(&variable) {
            phrase = phrase.replace(&variable, &as_group(grammar));
        }
    }
    phrase
}

/// The variables of the INI file, or `None` when the file cannot be read
/// or has more than [`MAX_INI_BYTES`] bytes.
fn read_ini(path: &Path) -> Option<HashMap<String, String>> {
    if fs::metadata(path).ok()?.len() > MAX_INI_BYTES as u64 {
        return None;
    }
    let bytes = fs::read(path).ok()?;
    (bytes.len() <= MAX_INI_BYTES).then(|| parse_ini(&String::from_utf8_lossy(&bytes)))
}

/// True if `;` appears outside every `[...]` group.
fn has_top_level_semicolon(s: &str) -> bool {
    let mut depth = 0usize;
    for c in s.chars() {
        match c {
            '[' => depth += 1,
            ']' => depth = depth.saturating_sub(1),
            ';' if depth == 0 => return true,
            _ => {}
        }
    }
    false
}

fn as_group(trigger: &str) -> String {
    if has_top_level_semicolon(trigger) {
        format!("[{trigger}]")
    } else {
        trigger.to_string()
    }
}

/// Build the map from variable key (without `EDCP_`) to trigger grammar.
fn parse_ini(text: &str) -> HashMap<String, String> {
    let mut vars = HashMap::new();
    let mut lists: HashMap<String, Vec<(String, String)>> = HashMap::new();
    let mut templates: Vec<(String, String)> = Vec::new();
    let mut current_list: Option<String> = None;

    for line in text.trim_start_matches('\u{feff}').lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("\\\\") {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let name = &line[1..line.len() - 1];
            current_list = (name.starts_with('<') && name.ends_with('>')).then(|| name.to_string());
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let Some((_, trigger)) = value.split_once('|') else {
            continue;
        };
        // A few lines have a third `|` field; it is not spoken grammar.
        let trigger = trigger.split('|').next().unwrap_or_default();
        let (key, trigger) = (key.trim().to_string(), trigger.trim().to_string());
        match &current_list {
            Some(list) => lists.entry(list.clone()).or_default().push((key, trigger)),
            None if key.contains('<') => templates.push((key, trigger)),
            None => {
                vars.insert(key, trigger);
            }
        }
    }

    // Templates may come before their list, so expand after reading the whole file.
    // A template with two or more list names gives no variable: the
    // expansion of one list would keep the name of the other list, and the
    // result would depend on the order of the lists.
    for (key, trigger) in &templates {
        let named: Vec<(&String, &Vec<(String, String)>)> = lists
            .iter()
            .filter(|(list, _)| key.contains(list.as_str()))
            .collect();
        if let [(list, items)] = named.as_slice() {
            for (item_key, item_trigger) in items.iter() {
                vars.insert(
                    key.replace(list.as_str(), item_key),
                    trigger.replace(list.as_str(), &as_group(item_trigger)),
                );
            }
        }
    }
    vars
}

fn substitute(phrase: &str, vars: &HashMap<String, String>) -> String {
    let mut out = String::with_capacity(phrase.len());
    let mut rest = phrase;
    while let Some(start) = rest.find(VAR_PREFIX) {
        out.push_str(&rest[..start]);
        let after = &rest[start..];
        let Some(end) = after.find('}') else {
            rest = after;
            break;
        };
        let name = &after[VAR_PREFIX.len()..end];
        match vars.get(name) {
            Some(trigger) => out.push_str(&as_group(trigger)),
            None => out.push_str(&after[..=end]),
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command_map::CommandClass;
    use std::path::PathBuf;

    fn ini() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/VoiceCommands.ini")
    }

    fn cmd(phrase: Option<&str>) -> Command {
        Command {
            phrase: phrase.map(String::from),
            token_template: "T".into(),
            class: CommandClass::Fixed,
            var_source: None,
            category: None,
            constants: HashMap::new(),
            number_vars: Vec::new(),
            ..Default::default()
        }
    }

    #[test]
    fn test_resolves_profile_variable() {
        let vars = HashMap::from([
            (
                "HarbourInfo".to_string(),
                "[depth at; berths in]".to_string(),
            ),
            ("Bare".to_string(), "one; two".to_string()),
        ]);
        let commands = vec![
            cmd(Some("{TXT:HarbourInfo} {TXT:System1}")),
            cmd(Some("{TXT:Bare} three")),
            cmd(Some("plain {TXT:Other}")),
            cmd(None),
        ];
        let phrases: Vec<Option<String>> = resolve_profile_vars(commands, &vars)
            .into_iter()
            .map(|c| c.phrase)
            .collect();
        assert_eq!(
            phrases,
            [
                Some("[depth at; berths in] {TXT:System1}".to_string()),
                Some("[one; two] three".to_string()),
                Some("plain {TXT:Other}".to_string()),
                None,
            ]
        );
    }

    fn resolve(phrase: Option<&str>) -> Option<String> {
        resolve_runtime_grammar(vec![cmd(phrase)], &ini())
            .commands
            .remove(0)
            .phrase
    }

    /// A blocking phrase gets the runtime grammar and the profile
    /// variables. A phrase that does not resolve or does not parse is
    /// skipped and counted.
    #[test]
    fn test_blocking_phrases_resolve_as_the_commands_do() {
        let phrase = |text: &str| BlockingPhrase {
            phrase: text.to_string(),
            class: LeftOutClass::NoWrite,
            command_type: CommandType::Full,
            composite_group: None,
        };
        let vars = HashMap::from([("Places".to_string(), "[quay; pier]".to_string())]);
        let deep = format!("{}x{}", "[".repeat(70), "]".repeat(70));
        let out = resolve_blocking(
            vec![
                phrase("{TXT:EDCP_Foo} now"),
                phrase("go to {TXT:Places}"),
                phrase("{TXT:EDCP_Missing} now"),
                phrase(&deep),
                phrase("plain words"),
            ],
            Some(&ini()),
            &vars,
        );
        let texts: Vec<&str> = out.phrases.iter().map(|b| b.phrase.as_str()).collect();
        assert_eq!(
            texts,
            ["[show;] foo now", "go to [quay; pier]", "plain words"]
        );
        assert_eq!(out.skipped, 2);
        // Without the file, each `{TXT:EDCP_` phrase is skipped.
        let out = resolve_blocking(vec![phrase("{TXT:EDCP_Foo} now")], None, &vars);
        assert_eq!((out.phrases.len(), out.skipped), (0, 1));
        // The commands that the number check took out block too.
        let removed = [
            RemovedCommand {
                phrase: "trim [1..3] {TXT:EDCP_Foo}".into(),
                token_template: "Trim={TXT:N}".into(),
                reason: NUMBER_TEXT_REASON.into(),
            },
            RemovedCommand {
                phrase: "((inner))".into(),
                token_template: "X".into(),
                reason: NUMBER_TEXT_REASON.into(),
            },
        ];
        let blocking = removed_as_blocking(&removed);
        assert_eq!(
            blocking,
            [phrase("trim [1..3] {TXT:EDCP_Foo}")].map(|b| BlockingPhrase {
                class: LeftOutClass::NotUnderstood,
                ..b
            })
        );
    }

    #[test]
    fn test_resolves_edcp_variable() {
        assert_eq!(
            resolve(Some("{TXT:EDCP_Foo}")).as_deref(),
            Some("[show;] foo")
        );
        assert_eq!(
            resolve(Some("plain phrase")).as_deref(),
            Some("plain phrase")
        );
    }

    /// A command with a constant system gets a fixed name slot: the spoken
    /// forms of the name and the system name.
    #[test]
    fn test_fixed_name_slot_for_a_constant_system() {
        let mut command = cmd(Some("{TXT:EDCP_SAILTO} {TXT:EDCP_TAMFERRY}"));
        command
            .constants
            .insert("SystemName".to_string(), "Alioth".to_string());
        let out = resolve_runtime_grammar(vec![command], &ini())
            .commands
            .remove(0);
        assert_eq!(
            out.phrase.as_deref(),
            Some("[sail out for; sail out to] {TXT:Fixed:TAMFERRY:Alioth}")
        );
        assert_eq!(
            out.fixed_vocab.get("Fixed:TAMFERRY:Alioth"),
            Some(&vec!["Tam Ferry".to_string(), "Alioth".to_string()])
        );
        // Without a constant system, the name variable is resolved as before.
        let plain = resolve(Some("{TXT:EDCP_SAILTO} {TXT:EDCP_TAMFERRY}"));
        assert_eq!(
            plain.as_deref(),
            Some("[sail out for; sail out to] Tam Ferry")
        );
        // A name variable that the file does not have: only the system name.
        let mut unknown = cmd(Some("go to {TXT:EDCP_NOSUCHNAME}"));
        unknown
            .constants
            .insert("SystemName".to_string(), "Sol".to_string());
        let out = resolve_runtime_grammar(vec![unknown], &ini())
            .commands
            .remove(0);
        assert_eq!(
            out.phrase.as_deref(),
            Some("go to {TXT:Fixed:NOSUCHNAME:Sol}")
        );
        assert_eq!(
            out.fixed_vocab["Fixed:NOSUCHNAME:Sol"],
            vec!["Sol".to_string()]
        );
    }

    fn fixed(phrase: &str, template: &str, system: &str) -> Command {
        let mut command = cmd(Some(phrase));
        command.token_template = template.to_string();
        command
            .constants
            .insert("SystemName".to_string(), system.to_string());
        command
    }

    /// Two commands with the same name variable and different systems keep
    /// their own names and tokens.
    #[test]
    fn test_shared_name_variable_with_different_systems() {
        use crate::aliases::AliasMap;
        use crate::matcher::Matcher;
        let template = "HarbourInfo:{TXT:SystemName}:sail out";
        let commands = resolve_runtime_grammar(
            vec![
                fixed("go to {TXT:EDCP_TAMFERRY}", template, "Alioth"),
                fixed("fly to {TXT:EDCP_TAMFERRY}", template, "Sol"),
            ],
            &ini(),
        )
        .commands;
        assert_ne!(commands[0].fixed_vocab, commands[1].fixed_vocab);
        let m = Matcher::new(commands, HashMap::new(), AliasMap::new(), 0.8);
        let token = |t: &str| m.match_text(t).map(|h| h.token);
        assert_eq!(
            token("go to alioth").as_deref(),
            Some("HarbourInfo:Alioth:sail out")
        );
        assert_eq!(
            token("fly to sol").as_deref(),
            Some("HarbourInfo:Sol:sail out")
        );
        assert_eq!(token("go to sol"), None);
        assert_eq!(token("fly to alioth"), None);
    }

    /// A name grammar with a range gives no spoken forms: the phrase keeps
    /// the name grammar, and the command gets no fixed slot.
    #[test]
    fn test_name_without_spoken_forms_keeps_the_phrase() {
        let out = resolve_runtime_grammar(
            vec![fixed("go to {TXT:EDCP_RANGENAME}", "T", "Sol")],
            &ini(),
        )
        .commands
        .remove(0);
        assert_eq!(out.phrase.as_deref(), Some("go to name [1..3]"));
        assert!(out.fixed_vocab.is_empty());
    }

    /// Some real lines have a third `|` field. Only the second field is the trigger.
    #[test]
    fn test_third_field_is_not_part_of_the_trigger() {
        assert_eq!(
            resolve(Some("{TXT:EDCP_TAMFERRY}")).as_deref(),
            Some("Tam Ferry")
        );
        assert_eq!(
            resolve(Some("{TXT:EDCP_TRAILING}")).as_deref(),
            Some("[show; hide;] only mine")
        );
    }

    #[test]
    fn test_mixed_phrase_keeps_other_variable_and_wraps_top_level_alternation() {
        assert_eq!(
            resolve(Some("{TXT:EDCP_SAILTO} {TXT:System96}")).as_deref(),
            Some("[sail out for; sail out to] {TXT:System96}")
        );
    }

    #[test]
    fn test_expands_templates_over_list_section() {
        assert_eq!(
            resolve(Some("{TXT:EDCP_BUYCUTTERMK4}")).as_deref(),
            Some("[best yard;] [to;] [buy; purchase;] [a;] Cutter [Mark Four;]")
        );
        assert_eq!(
            resolve(Some("{TXT:EDCP_BUYDORY}")).as_deref(),
            Some("[best yard;] [to;] [buy; purchase;] [a;] Dory")
        );
        assert_eq!(
            resolve(Some("{TXT:EDCP_BUYSCHOONER}")).as_deref(),
            Some("[best yard;] [to;] [buy; purchase;] [a;] [Topsail Schooner; Schooner]")
        );
    }

    #[test]
    fn test_unresolvable_inputs_stay_unchanged() {
        assert_eq!(
            resolve(Some("{TXT:EDCP_NOPE}")).as_deref(),
            Some("{TXT:EDCP_NOPE}")
        );
        assert_eq!(
            resolve(Some("{TXT:EDCP_NOLISTX}")).as_deref(),
            Some("{TXT:EDCP_NOLISTX}")
        );
        assert_eq!(resolve(None), None);
        let missing = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/absent.ini");
        let out = resolve_runtime_grammar(vec![cmd(Some("{TXT:EDCP_Foo}"))], &missing).commands;
        assert_eq!(out[0].phrase.as_deref(), Some("{TXT:EDCP_Foo}"));
    }

    /// A template with two list names gives no variable: the result must
    /// not depend on the order of the lists. A file larger than the limit
    /// reads as a file that does not read.
    #[test]
    fn test_two_list_names_and_a_large_file_give_no_grammar() {
        let text = "[Grammar]\n\
                    PAIR<Mast><Sail> = x|raise <Mast> <Sail>\n\
                    ONE<Mast> = x|raise <Mast>\n\
                    [<Mast>]\nFORE = x|fore mast\nMAIN = x|main mast\n\
                    [<Sail>]\nJIB = x|jib\n";
        for _ in 0..20 {
            let vars = parse_ini(text);
            assert_eq!(
                vars.get("ONEFORE").map(String::as_str),
                Some("raise fore mast")
            );
            assert!(vars.keys().all(|k| !k.starts_with("PAIR")), "{vars:?}");
        }
        let dir = tempfile::tempdir().unwrap();
        let ini = dir.path().join("VoiceCommands.ini");
        let mut big = String::from("SAILTO = x|sail out\n");
        big.push_str(&" ".repeat(MAX_INI_BYTES));
        std::fs::write(&ini, &big).unwrap();
        assert!(RuntimeGrammar::read(&ini).is_none());
        std::fs::write(&ini, "SAILTO = x|sail out\n").unwrap();
        let grammar = RuntimeGrammar::read(&ini).unwrap();
        let out = resolve_runtime_grammar_from(
            vec![cmd(Some("{TXT:EDCP_SAILTO} now"))],
            Some(&grammar),
            &HashMap::new(),
        );
        assert_eq!(out.commands[0].phrase.as_deref(), Some("sail out now"));
        let blocking = resolve_blocking_from(
            vec![BlockingPhrase {
                phrase: "{TXT:EDCP_SAILTO} later".into(),
                class: LeftOutClass::NoWrite,
                command_type: CommandType::Full,
                composite_group: None,
            }],
            Some(&grammar),
            &HashMap::new(),
        );
        assert_eq!(blocking.phrases[0].phrase, "sail out later");
    }

    /// The number check runs on the resolved phrase of each number command.
    #[test]
    fn test_number_command_with_number_text_in_the_grammar_leaves_the_list() {
        let dir = tempfile::tempdir().unwrap();
        let ini = dir.path().join("VoiceCommands.ini");
        fs::write(
            &ini,
            "[Lamps]\r\n\
             TWIN = Lamp rate|[lamp; lamp-] rate [10..30, 10]\r\n\
             WHARF = Wharf lamp|wharf-side lamp [1..9]\r\n\
             ZERO = Lamp with zero|lamp [0;] [1..50]\r\n\
             POINT = Lamp point|lamp. [1..9]\r\n",
        )
        .unwrap();
        let number = |phrase: &str| {
            let mut c = cmd(Some(phrase));
            c.token_template = "Lamp={TXT:Level}".into();
            c.number_vars = vec!["Level".into()];
            c
        };
        let out = resolve_runtime_grammar(
            vec![
                number("{TXT:EDCP_TWIN}"),
                number("{TXT:EDCP_WHARF}"),
                number("{TXT:EDCP_ZERO}"),
                number("{TXT:EDCP_POINT}"),
                // The same grammar without a number variable stays.
                cmd(Some("{TXT:EDCP_WHARF}")),
            ],
            &ini,
        );
        let phrases: Vec<_> = out
            .commands
            .iter()
            .map(|c| c.phrase.as_deref().unwrap())
            .collect();
        assert_eq!(
            phrases,
            [
                "[lamp; lamp-] rate [10..30, 10]",
                "lamp [0;] [1..50]",
                "wharf-side lamp [1..9]"
            ]
        );
        assert!(out.commands[0].number_vars == ["Level"]);
        assert!(out.commands[2].number_vars.is_empty());
        let removed: Vec<_> = out.removed.iter().map(|r| r.phrase.as_str()).collect();
        assert_eq!(removed, ["{TXT:EDCP_WHARF}", "{TXT:EDCP_POINT}"]);
        assert!(out
            .removed
            .iter()
            .all(|r| r.reason == NUMBER_TEXT_REASON && r.token_template == "Lamp={TXT:Level}"));
        // Without the file, the check runs on the phrase as it is.
        let none = resolve_runtime_grammar(
            vec![number("lamp. [0;] [1..9]"), number("{TXT:EDCP_WHARF}")],
            &dir.path().join("absent.ini"),
        );
        assert_eq!(none.commands.len(), 1);
        assert_eq!(none.removed[0].phrase, "lamp. [0;] [1..9]");
    }

    #[test]
    fn test_comment_line_is_not_a_variable() {
        // The fixture holds a comment line in the shape of the real one, `\\LAMPNAME = Text|lamp grammar`.
        assert_eq!(
            resolve(Some("{TXT:EDCP_LAMPNAME}")).as_deref(),
            Some("{TXT:EDCP_LAMPNAME}")
        );
    }
}
