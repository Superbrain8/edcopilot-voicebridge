//! `voicebridge commands`: shows which commands the matcher can use.

use std::path::Path;
use std::process::ExitCode;

use edcopilot_voicebridge_core::{
    load_aliases, load_fillers_with_problems, load_synonyms_with_problems, Command, CommandType,
    Fillers, Synonyms,
};

use crate::config::{command_state, load_commands, CommandState, Config};

/// Longest phrase text in one line of the list.
const MAX_PHRASE_CHARS: usize = 160;

/// One line of the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub phrase: String,
    pub token: String,
    pub category: String,
    pub state: CommandState,
    /// True for a phrase from an alias file.
    pub alias: bool,
}

/// The prefixes and the suffixes of one composite group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompositeView {
    pub group: String,
    /// Prefix phrase and the variables it sets, as `name = value` text.
    pub prefixes: Vec<(String, String)>,
    /// Suffix phrase, token template and state.
    pub suffixes: Vec<(String, String, CommandState)>,
    /// How the suffix conditions read the prefix, from the first suffix.
    pub rules: String,
}

/// One view for each composite group, in the order of the map.
pub fn composite_views(commands: &[Command]) -> Vec<CompositeView> {
    let mut views: Vec<CompositeView> = Vec::new();
    for command in commands.iter().filter(|c| c.is_composite_part()) {
        let group = command.composite_group.clone().unwrap_or_default();
        let index = match views.iter().position(|v| v.group == group) {
            Some(index) => index,
            None => {
                views.push(CompositeView {
                    group,
                    prefixes: Vec::new(),
                    suffixes: Vec::new(),
                    rules: String::new(),
                });
                views.len() - 1
            }
        };
        let view = &mut views[index];
        let phrase = one_line(command.phrase.as_deref().unwrap_or(""));
        match command.command_type {
            CommandType::Prefix => {
                let mut sets: Vec<String> = command
                    .sets
                    .iter()
                    .map(|(k, v)| format!("{k} = {v}"))
                    .collect();
                sets.sort();
                view.prefixes.push((phrase, sets.join(", ")));
            }
            CommandType::Suffix => {
                if view.rules.is_empty() {
                    view.rules = command
                        .prefix_rules
                        .iter()
                        .flat_map(|rule| {
                            rule.set.iter().map(move |(name, value)| {
                                format!("{name} = {value} if the prefix has \"{}\"", rule.contains)
                            })
                        })
                        .collect::<Vec<_>>()
                        .join("; ");
                }
                view.suffixes
                    .push((phrase, command.effective_template(), command_state(command)));
            }
            CommandType::Full => {}
        }
    }
    views
}

/// The lines of one composite group. With a filter, the group shows when the
/// filter is in its name or a prefix (all suffixes), or in a suffix line (only
/// those suffixes). A group with no prefix or no suffix that works says so.
pub fn composite_lines(view: &CompositeView, filter: &str, all: bool) -> Vec<String> {
    let head = format!(
        "{} {}",
        view.group,
        view.prefixes
            .iter()
            .map(|(p, s)| format!("{p} {s}"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    let whole = filter_accepts(&head, filter);
    let suffix_lines: Vec<String> = view
        .suffixes
        .iter()
        .filter(|(_, _, state)| all || *state == CommandState::CompositePart)
        .map(|(phrase, token, state)| {
            let mut text = format!("  suffix: {phrase}  ->  {token}");
            if *state == CommandState::UnresolvedGrammar {
                text.push_str("  [does not work: grammar not found in the voice commands file]");
            }
            text
        })
        .filter(|line| whole || filter_accepts(line, filter))
        .collect();
    if suffix_lines.is_empty() {
        return Vec::new();
    }
    let mut lines = vec![format!(
        "\n== Composite group \"{}\": say one prefix and one suffix as one sentence",
        view.group
    )];
    if view.prefixes.is_empty() {
        lines.push("  [does not work: the group has no prefix]".to_string());
    }
    if !view.rules.is_empty() {
        lines.push(format!("  rules: {}", view.rules));
    }
    for (phrase, sets) in &view.prefixes {
        lines.push(format!("  prefix: {phrase}  (sets {sets})"));
    }
    lines.extend(suffix_lines);
    lines
}

/// True when each word of `filter` is in `text`, without regard to case.
fn filter_accepts(text: &str, filter: &str) -> bool {
    let text = text.to_lowercase();
    filter
        .split_whitespace()
        .all(|word| text.contains(&word.to_lowercase()))
}

/// One line for each synonym whose key or values have each filter word:
/// `"head for"  =  "sail out for" | "sail out to"  (synonym)`.
pub fn synonym_lines(table: &Synonyms, filter: &str) -> Vec<String> {
    table
        .entries()
        .iter()
        .filter(|e| filter_accepts(&format!("{} {}", e.key, e.values.join(" ")), filter))
        .map(|e| {
            let values: Vec<String> = e.values.iter().map(|v| format!("\"{v}\"")).collect();
            format!("\"{}\"  =  {}  (synonym)", e.key, values.join(" | "))
        })
        .collect()
}

/// One line with all filler words when the filter is empty or each filter
/// word is in a filler word: `please, the, uh  (filler)`.
pub fn filler_lines(table: &Fillers, filter: &str) -> Vec<String> {
    let words: Vec<&str> = table.words();
    let shown = !words.is_empty()
        && filter
            .split_whitespace()
            .map(str::to_lowercase)
            .all(|f| words.iter().any(|w| w.contains(f.as_str())));
    if shown {
        vec![format!("{}  (filler)", words.join(", "))]
    } else {
        Vec::new()
    }
}

/// The phrase with each fixed name slot written as its names:
/// `[Lantern Rock; Gull Isle]` for `{TXT:Fixed:LANTERNROCK}`.
fn shown_phrase(command: &Command) -> String {
    let mut phrase = command.phrase.clone().unwrap_or_default();
    let mut slots: Vec<_> = command.fixed_vocab.iter().collect();
    slots.sort();
    for (slot, names) in slots {
        phrase = phrase.replace(
            &format!("{{TXT:{slot}}}"),
            &format!("[{}]", names.join("; ")),
        );
    }
    phrase
}

/// Builds the list: one entry for each full command, then one for each alias
/// phrase of a command that works. Sorted by category, then by phrase.
/// Prefixes and suffixes of composite commands are in `composite_views`.
pub fn entries(commands: &[Command], aliases: &[(String, Vec<String>)]) -> Vec<Entry> {
    let mut list: Vec<Entry> = commands
        .iter()
        .filter(|c| !c.is_composite_part())
        .map(|c| Entry {
            phrase: one_line(&shown_phrase(c)),
            token: c.effective_template(),
            category: c.category.clone().unwrap_or_default(),
            state: command_state(c),
            alias: false,
        })
        .collect();
    for (template, phrases) in aliases {
        let Some(command) = commands
            .iter()
            .find(|c| &c.token_template == template && command_state(c) == CommandState::Works)
        else {
            continue;
        };
        for phrase in phrases {
            list.push(Entry {
                phrase: one_line(phrase),
                token: command.effective_template(),
                category: command.category.clone().unwrap_or_default(),
                state: CommandState::Works,
                alias: true,
            });
        }
    }
    list.sort_by(|a, b| {
        (a.category.to_lowercase(), a.phrase.to_lowercase())
            .cmp(&(b.category.to_lowercase(), b.phrase.to_lowercase()))
    });
    list
}

/// True when each word of `filter` is in the phrase, the token or the
/// category, without regard to case. An empty filter accepts all entries.
pub fn matches_filter(entry: &Entry, filter: &str) -> bool {
    let text = format!("{} {} {}", entry.phrase, entry.token, entry.category);
    filter_accepts(&text, filter)
}

/// Why a command cannot fire, as the list shows it. `None` for a command
/// that works.
pub fn reason(state: CommandState) -> Option<&'static str> {
    match state {
        CommandState::Works => None,
        CommandState::NoPhrase => Some("does not work: no phrase"),
        CommandState::UnresolvedGrammar => {
            Some("does not work: grammar not found in the voice commands file")
        }
        CommandState::NeedsValue => {
            Some("does not work: needs a value that the phrase does not give")
        }
        CommandState::CompositePart => {
            Some("only together with the other part of its composite group")
        }
    }
}

/// The text of one entry. A command that cannot fire gets the reason.
pub fn line(entry: &Entry) -> String {
    let mut text = format!("{}  ->  {}", entry.phrase, entry.token);
    if entry.alias {
        text.push_str("  (alias)");
    }
    if let Some(reason) = reason(entry.state) {
        text.push_str(&format!("  [{reason}]"));
    }
    text
}

/// The data of the list, loaded one time. The console prints it, and the
/// user interface filters it again for each change of the filter.
pub struct ListData {
    /// The full commands and the aliases, as `entries` gives them.
    pub entries: Vec<Entry>,
    pub views: Vec<CompositeView>,
    pub synonyms: Synonyms,
    /// The problems of the synonym file.
    pub synonym_problems: Vec<String>,
    pub fillers: Fillers,
    /// The problems of the filler file.
    pub filler_problems: Vec<String>,
    /// True in `always_listening` mode: the filler words are off.
    pub fillers_off: bool,
}

impl ListData {
    /// Loads the commands of the EDCoPilot profile, the aliases, the
    /// synonyms and the filler words. A missing profile is an error.
    pub fn load(config: &Config) -> anyhow::Result<ListData> {
        Ok(ListData::from_commands(
            config,
            &load_commands(config)?.commands,
        ))
    }

    /// `load` with the commands of `load_commands`, so that a caller that
    /// also builds the pipeline loads the profile one time.
    pub fn from_commands(config: &Config, commands: &[Command]) -> ListData {
        let mut aliases: Vec<(String, Vec<String>)> =
            load_aliases(&config.data.aliases_dir).into_iter().collect();
        aliases.sort();
        let (synonyms, synonym_problems) = load_synonyms_with_problems(&config.data.synonyms);
        let (fillers, filler_problems) = load_fillers_with_problems(&config.data.fillers);
        ListData {
            entries: entries(commands, &aliases),
            views: composite_views(commands),
            synonyms,
            synonym_problems,
            fillers,
            filler_problems,
            fillers_off: config.microphone.mode == "always_listening",
        }
    }

    /// The number of entries that work.
    pub fn working(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| e.state == CommandState::Works)
            .count()
    }

    /// The entries that the list shows for the filter. Without `all`, only
    /// the entries that work.
    pub fn shown(&self, filter: &str, all: bool) -> Vec<&Entry> {
        self.entries
            .iter()
            .filter(|e| (all || e.state == CommandState::Works) && matches_filter(e, filter))
            .collect()
    }

    /// The last line of the list.
    pub fn summary(&self, shown: usize) -> String {
        format!(
            "{shown} shown. {} of {} phrases work (commands and aliases). \
             {} composite groups. {} synonyms. {} filler words.",
            self.working(),
            self.entries.len(),
            self.views.len(),
            self.synonyms.len(),
            self.fillers.len()
        )
    }

    /// The heading of the filler words, with their state.
    pub fn filler_heading(&self) -> String {
        let state = if self.fillers_off {
            "off in always_listening mode"
        } else {
            "dropped only when nothing else matches"
        };
        format!("== Filler words ({state})")
    }
}

/// One line, no control characters, cut to `MAX_PHRASE_CHARS`.
fn one_line(text: &str) -> String {
    let clean: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let clean = clean.trim();
    if clean.chars().count() > MAX_PHRASE_CHARS {
        let cut: String = clean.chars().take(MAX_PHRASE_CHARS).collect();
        format!("{cut}...")
    } else {
        clean.to_string()
    }
}

/// `voicebridge commands`: prints the commands that work, by category.
/// With `all` it also prints the commands that cannot fire, with the reason.
pub fn commands(config_path: &Path, filter: &str, all: bool) -> anyhow::Result<ExitCode> {
    let config = Config::load(config_path)?;
    let data = ListData::load(&config)?;
    let mut shown = 0;
    let mut category: Option<&str> = None;
    for entry in data.shown(filter, all) {
        if category != Some(entry.category.as_str()) {
            category = Some(entry.category.as_str());
            let name = if entry.category.is_empty() {
                "(no category)"
            } else {
                entry.category.as_str()
            };
            println!("\n== {name}");
        }
        println!("{}", line(entry));
        shown += 1;
    }
    for view in &data.views {
        for text in composite_lines(view, filter, all) {
            println!("{text}");
        }
    }
    for problem in &data.synonym_problems {
        eprintln!("warning: {problem}");
    }
    let synonym_lines = synonym_lines(&data.synonyms, filter);
    if !synonym_lines.is_empty() {
        println!("\n== Synonyms (they work in every command)");
        for text in &synonym_lines {
            println!("{text}");
        }
    }
    for problem in &data.filler_problems {
        eprintln!("warning: {problem}");
    }
    let filler_lines = filler_lines(&data.fillers, filter);
    if !filler_lines.is_empty() {
        println!("\n{}", data.filler_heading());
        for text in &filler_lines {
            println!("{text}");
        }
    }
    println!("\n{}", data.summary(shown));
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use edcopilot_voicebridge_core::CommandClass;
    use std::collections::HashMap;

    fn command(phrase: Option<&str>, token: &str, class: CommandClass, category: &str) -> Command {
        Command {
            phrase: phrase.map(str::to_string),
            token_template: token.to_string(),
            class,
            var_source: None,
            category: Some(category.to_string()),
            constants: HashMap::new(),
            number_vars: Vec::new(),
            ..Default::default()
        }
    }

    fn sample() -> Vec<Command> {
        vec![
            command(
                Some("open chart log"),
                "OpenChartLogPanel",
                CommandClass::Fixed,
                "Panels",
            ),
            command(None, "NoPhraseToken", CommandClass::Fixed, "Panels"),
            command(
                Some("{TXT:EDCP_X}"),
                "Unresolved",
                CommandClass::Fixed,
                "Panels",
            ),
            command(
                Some("set volume"),
                "Vol={TXT:v}",
                CommandClass::DynamicValue,
                "Audio",
            ),
        ]
    }

    #[test]
    fn test_entries_carry_the_state_and_are_sorted_by_category() {
        let list = entries(&sample(), &[]);
        assert_eq!(list.len(), 4);
        assert_eq!(list[0].category, "Audio");
        assert_eq!(list[0].state, CommandState::NeedsValue);
        let states: Vec<_> = list.iter().map(|e| e.state).collect();
        assert!(states.contains(&CommandState::Works));
        assert!(states.contains(&CommandState::NoPhrase));
        assert!(states.contains(&CommandState::UnresolvedGrammar));
    }

    #[test]
    fn test_alias_phrases_are_listed_only_for_a_command_that_works() {
        let aliases = vec![
            (
                "OpenChartLogPanel".to_string(),
                vec!["kartenbuch".to_string()],
            ),
            ("Unresolved".to_string(), vec!["nicht da".to_string()]),
            ("NotInTheMap".to_string(), vec!["fremd".to_string()]),
        ];
        let list = entries(&sample(), &aliases);
        let alias: Vec<_> = list.iter().filter(|e| e.alias).collect();
        assert_eq!(alias.len(), 1);
        assert_eq!(alias[0].phrase, "kartenbuch");
        assert_eq!(alias[0].token, "OpenChartLogPanel");
    }

    #[test]
    fn test_filter_needs_each_word_and_ignores_case() {
        let list = entries(&sample(), &[]);
        let entry = list
            .iter()
            .find(|e| e.token == "OpenChartLogPanel")
            .unwrap();
        assert!(matches_filter(entry, ""));
        assert!(matches_filter(entry, "CHART log"));
        assert!(matches_filter(entry, "panels chart"));
        assert!(!matches_filter(entry, "chart course"));
    }

    #[test]
    fn test_line_gives_the_reason_for_a_command_that_cannot_fire() {
        let list = entries(&sample(), &[]);
        let works = list
            .iter()
            .find(|e| e.state == CommandState::Works)
            .unwrap();
        assert_eq!(line(works), "open chart log  ->  OpenChartLogPanel");
        let needs = list
            .iter()
            .find(|e| e.state == CommandState::NeedsValue)
            .unwrap();
        assert!(line(needs).contains("does not work: needs a value"));
    }

    fn composite_sample() -> Vec<Command> {
        let prefix = Command {
            command_type: CommandType::Prefix,
            composite_group: Some("trade lookup".to_string()),
            sets: HashMap::from([("offer_scope".to_string(), "Dock:".to_string())]),
            ..command(
                Some("[best;] local [sell; buy]"),
                "",
                CommandClass::Fixed,
                "P",
            )
        };
        let suffix = |phrase: &str, token: &str| Command {
            command_type: CommandType::Suffix,
            composite_group: Some("trade lookup".to_string()),
            prefix_rules: vec![edcopilot_voicebridge_core::PrefixRule {
                contains: "sell".to_string(),
                set: HashMap::from([("TradeSide".to_string(), "Sell".to_string())]),
            }],
            ..command(Some(phrase), token, CommandClass::DynamicValue, "S")
        };
        let mut commands = sample();
        commands.push(prefix);
        commands.push(suffix(
            "[price for;] cork",
            "{TXT:TradeSide}{TXT:offer_scope}Cork",
        ));
        commands.push(suffix(
            "{TXT:EDCP_RATE_TEAK}",
            "{TXT:TradeSide}{TXT:offer_scope}Teak",
        ));
        commands
    }

    #[test]
    fn test_composite_parts_are_not_listed_as_commands() {
        let list = entries(&composite_sample(), &[]);
        assert_eq!(list.len(), 4);
        assert!(list.iter().all(|e| !e.token.contains("TradeSide")));
    }

    #[test]
    fn test_composite_group_lists_prefixes_then_suffixes() {
        let views = composite_views(&composite_sample());
        assert_eq!(views.len(), 1);
        let lines = composite_lines(&views[0], "", false);
        assert!(lines[0].contains("Composite group \"trade lookup\""));
        assert_eq!(
            lines[1],
            "  rules: TradeSide = Sell if the prefix has \"sell\""
        );
        assert_eq!(
            lines[2],
            "  prefix: [best;] local [sell; buy]  (sets offer_scope = Dock:)"
        );
        assert_eq!(
            lines[3],
            "  suffix: [price for;] cork  ->  {TXT:TradeSide}{TXT:offer_scope}Cork"
        );
        // The unresolved suffix shows only with `all`, with the reason.
        assert_eq!(lines.len(), 4);
        let all = composite_lines(&views[0], "", true);
        assert_eq!(all.len(), 5);
        assert!(all[4].contains("does not work: grammar not found"));
    }

    #[test]
    fn test_composite_filter_on_a_prefix_or_a_suffix() {
        let views = composite_views(&composite_sample());
        // A prefix word shows the whole group.
        assert_eq!(composite_lines(&views[0], "LOCAL", false).len(), 4);
        // A suffix word shows only that suffix, with the prefixes.
        let cork = composite_lines(&views[0], "cork", false);
        assert_eq!(cork.len(), 4);
        assert!(cork[3].contains("cork"));
        assert!(composite_lines(&views[0], "chart", false).is_empty());
    }

    #[test]
    fn test_synonym_lines_and_their_filter() {
        let table = Synonyms::from_pairs([
            ("navigate to", vec!["sail out to", "sail out for"]),
            ("navigate", vec!["sail out for"]),
        ]);
        let all = synonym_lines(&table, "");
        assert_eq!(
            all,
            vec![
                "\"navigate to\"  =  \"sail out to\" | \"sail out for\"  (synonym)",
                "\"navigate\"  =  \"sail out for\"  (synonym)",
            ]
        );
        assert_eq!(synonym_lines(&table, "out to").len(), 1);
        assert_eq!(synonym_lines(&table, "NAVIGATE").len(), 2);
        assert!(synonym_lines(&table, "chart log").is_empty());
    }

    #[test]
    fn test_filler_lines_and_their_filter() {
        let table = Fillers::from_lists(["the"], ["uh", "please"]);
        assert_eq!(filler_lines(&table, ""), vec!["please, the, uh  (filler)"]);
        assert_eq!(filler_lines(&table, "PLEASE").len(), 1);
        assert!(filler_lines(&table, "chart log").is_empty());
        assert!(filler_lines(&Fillers::default(), "").is_empty());
    }

    #[test]
    fn test_fixed_name_slot_is_shown_as_its_names() {
        let mut c = command(
            Some("[sail out for; sail out to] {TXT:Fixed:HALVARD}"),
            "HarbourInfo:Siris:sail out",
            CommandClass::DynamicName,
            "Engineers",
        );
        c.fixed_vocab.insert(
            "Fixed:HALVARD".to_string(),
            vec!["Halvard Crane".to_string(), "Siris".to_string()],
        );
        let list = entries(&[c], &[]);
        assert_eq!(
            list[0].phrase,
            "[sail out for; sail out to] [Halvard Crane; Siris]"
        );
    }

    #[test]
    fn test_reason_is_the_text_in_the_brackets_of_the_line() {
        let list = entries(&sample(), &[]);
        for entry in &list {
            match reason(entry.state) {
                None => assert!(!line(entry).contains('['), "{}", line(entry)),
                Some(text) => assert!(
                    line(entry).ends_with(&format!("  [{text}]")),
                    "{}",
                    line(entry)
                ),
            }
        }
        assert_eq!(reason(CommandState::Works), None);
        assert_eq!(
            reason(CommandState::NoPhrase),
            Some("does not work: no phrase")
        );
    }

    /// A configuration with the invented EDCoPilot folder of the fixture
    /// and the word tables of the repository. The program reads the
    /// profile of the folder.
    fn repository_config(dir: &tempfile::TempDir) -> Config {
        crate::commands_source::test_support::fixture_config(dir.path(), "")
    }

    #[test]
    fn test_list_data_gives_the_rows_and_the_counts_of_the_console() {
        let dir = tempfile::tempdir().unwrap();
        let config = repository_config(&dir);
        let data = ListData::load(&config).unwrap();
        // One row for each loaded fixture command that is not a part of a
        // composite command.
        let counts = edcopilot_voicebridge_core::profile::fixture::fixture_counts();
        assert_eq!(data.entries.len(), counts.loaded - counts.composite_parts);
        assert_eq!(data.views.len(), 1);
        let working = data
            .entries
            .iter()
            .filter(|e| e.state == CommandState::Works)
            .count();
        assert_eq!(data.working(), working);
        // Some rows do not work (for example the slots with no name).
        assert!(working < data.entries.len());
        let shown = data.shown("tide log", false);
        assert!(!shown.is_empty());
        assert!(shown.iter().all(|e| matches_filter(e, "tide log")));
        assert!(shown.iter().all(|e| e.state == CommandState::Works));
        assert!(data.shown("", true).len() == data.entries.len());
        assert_eq!(
            data.summary(7),
            format!(
                "7 shown. {working} of {} phrases work (commands and aliases). \
                 {} composite groups. {} synonyms. {} filler words.",
                data.entries.len(),
                data.views.len(),
                data.synonyms.len(),
                data.fillers.len()
            )
        );
        assert_eq!(
            data.filler_heading(),
            "== Filler words (dropped only when nothing else matches)"
        );
    }

    /// The window loads the commands one time for the list and the pipeline.
    #[test]
    fn test_one_load_of_the_commands_gives_the_list_and_the_pipeline() {
        let dir = tempfile::tempdir().unwrap();
        let config = repository_config(&dir);
        let first = load_commands(&config).unwrap();
        let shared = ListData::from_commands(&config, &first.commands);
        let loaded = ListData::load(&config).unwrap();
        assert_eq!(shared.entries, loaded.entries);
        assert_eq!(shared.views, loaded.views);
        let (_, alone) = crate::config::build_pipeline(&config).unwrap();
        // The report has the warnings and the profile report of the load.
        let (_, with) = crate::config::build_pipeline_with_commands(&config, first);
        assert!(alone.profile.records > 0);
        assert_eq!(format!("{alone:?}"), format!("{with:?}"));
    }

    #[test]
    fn test_list_data_without_a_profile_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let config = repository_config(&dir);
        assert!(ListData::load(&config).is_ok());
        // No profile in the EDCoPilot folder.
        std::fs::remove_file(&config.edcopilot.profile).unwrap();
        assert!(ListData::load(&config).is_err());
    }

    #[test]
    fn test_phrase_is_one_line_and_has_a_length_limit() {
        let long = format!("a\nb {}", "x".repeat(300));
        let text = one_line(&long);
        assert!(!text.contains('\n'));
        assert!(text.ends_with("..."));
        assert_eq!(text.chars().count(), MAX_PHRASE_CHARS + 3);
    }
}
