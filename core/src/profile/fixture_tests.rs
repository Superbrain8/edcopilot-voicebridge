//! The tests of the synthetic profile. Each property group has its own
//! test, so that a change of the fixture that drops a property fails.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::*;
use crate::profile::{build_map, BlockingPhrase, BuiltMap, FormatNote, LeftOutClass, MIN_RECORDS};
use crate::{
    load_dynamic_vocab, load_fillers_with_problems, load_sound_alikes, removed_as_blocking,
    resolve_blocking, resolve_profile_vars, resolve_runtime_grammar, AliasMap, BlockReason,
    CommandClass, CommandType, Match, MatchResult, Matcher, Pipeline, Writer,
};

fn built() -> BuiltMap {
    build_map(&fixture_profile()).unwrap()
}

fn data(file: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("data")
        .join(file)
}

/// The commands as the program loads them from the fixture folder (the
/// runtime grammar of the INI, then the profile variables), and the
/// blocking phrases as the program loads them: those of the build and
/// those of the commands that the number check takes out, with the same
/// grammar.
fn loaded_with_blocking(dir: &Path) -> (Vec<Command>, Vec<BlockingPhrase>) {
    let built = build_map(&std::fs::read(dir.join("EDCoPilot-Profile.vap")).unwrap()).unwrap();
    let ini = dir.join("working").join("EDCoPilot.VoiceCommands.ini");
    let out = resolve_runtime_grammar(built.commands, &ini);
    let mut blocking = built.blocking;
    blocking.extend(removed_as_blocking(&out.removed));
    let blocking = resolve_blocking(blocking, Some(&ini), &built.profile_vars);
    (
        resolve_profile_vars(out.commands, &built.profile_vars),
        blocking.phrases,
    )
}

/// The runtime grammar check takes out the one number command whose
/// grammar has a minus sign outside its ranges.
#[test]
fn test_fixture_number_grammar_with_a_minus_leaves_the_list() {
    let dir = tempfile::tempdir().unwrap();
    write_fixture_edcopilot(dir.path());
    let built =
        build_map(&std::fs::read(dir.path().join("EDCoPilot-Profile.vap")).unwrap()).unwrap();
    let ini = dir
        .path()
        .join("working")
        .join("EDCoPilot.VoiceCommands.ini");
    let out = resolve_runtime_grammar(built.commands.clone(), &ini);
    let removed: Vec<_> = out.removed.iter().map(|r| r.phrase.as_str()).collect();
    assert_eq!(removed, ["{TXT:EDCP_WICKLAMP}"]);
    assert_eq!(out.removed[0].reason, crate::NUMBER_TEXT_REASON);
    assert_eq!(out.commands.len() + 1, built.commands.len());
}

/// The fixture folder, its loaded commands, its blocking phrases, and its
/// vocabulary.
struct Folder {
    _dir: tempfile::TempDir,
    commands: Vec<Command>,
    blocking: Vec<BlockingPhrase>,
    vocab: HashMap<String, Vec<String>>,
}

fn folder() -> Folder {
    let dir = tempfile::tempdir().unwrap();
    write_fixture_edcopilot(dir.path());
    let (commands, blocking) = loaded_with_blocking(dir.path());
    let vocab = load_dynamic_vocab(&dir.path().join("Dynamic var files"));
    Folder {
        _dir: dir,
        commands,
        blocking,
        vocab,
    }
}

/// A pipeline on the loaded fixture with the default sound-alike and
/// filler tables, and the blocking phrases as the program loads them. The
/// writer path is never written: the tests only match.
fn pipeline() -> Pipeline {
    pipeline_with(true)
}

/// `pipeline`, with or without the blocking phrases.
fn pipeline_with(blocking: bool) -> Pipeline {
    let f = folder();
    let matcher = Matcher::new(f.commands, f.vocab, AliasMap::new(), 0.8);
    let matcher = if blocking {
        matcher.with_blocking(&f.blocking)
    } else {
        matcher
    };
    let (fillers, problems) = load_fillers_with_problems(&data("fillers.toml"));
    assert!(problems.is_empty(), "{problems:?}");
    let writer = Writer::new(std::env::temp_dir().join("voicebridge-never-written.txt"));
    Pipeline::new(matcher, writer)
        .with_sound_alikes(load_sound_alikes(&data("sound_alikes.toml")).unwrap())
        .with_fillers(fillers)
}

fn hit(p: &Pipeline, text: &str) -> Match {
    p.match_text(text)
        .unwrap_or_else(|| panic!("no match for {text:?}"))
}

fn token(p: &Pipeline, text: &str) -> Option<String> {
    p.match_text(text).map(|m| m.token)
}

fn by_phrase<'a>(commands: &'a [Command], phrase: &str) -> &'a Command {
    commands
        .iter()
        .find(|c| c.phrase.as_deref() == Some(phrase))
        .unwrap_or_else(|| panic!("no command {phrase:?}"))
}

// ---------------------------------------------------------------------
// The tests of the plan. The test that the fixture holds no real text is
// `test_fixture_holds_no_text_of_the_real_profile` in `tests/live_probes.rs`.
// ---------------------------------------------------------------------

/// The counts of the fixture, computed with the load steps of the program
/// in a temporary fixture folder.
fn computed_fixture_counts() -> FixtureCounts {
    let dir = tempfile::tempdir().unwrap();
    write_fixture_edcopilot(dir.path());
    let built = built();
    let ini = dir
        .path()
        .join("working")
        .join("EDCoPilot.VoiceCommands.ini");
    let resolved = crate::resolve_runtime_grammar_with(
        built.commands.clone(),
        Some(&ini),
        &built.profile_vars,
    );
    let mut blocking = built.blocking.clone();
    blocking.extend(removed_as_blocking(&resolved.removed));
    let blocking = resolve_blocking(blocking, Some(&ini), &built.profile_vars);
    let vocab = load_dynamic_vocab(&dir.path().join("Dynamic var files"));
    FixtureCounts {
        commands_in_profile: built.report.commands_in_profile,
        records: built.report.records,
        left_out: built.report.left_out.len(),
        removed_by_number_check: resolved.removed.len(),
        loaded: resolved.commands.len(),
        composite_parts: resolved
            .commands
            .iter()
            .filter(|c| c.command_type != CommandType::Full)
            .count(),
        blocking: built.blocking.len(),
        blocking_loaded: blocking.phrases.len(),
        blocking_skipped: blocking.skipped,
        vocabulary_files: VOCABULARY.len(),
        vocabulary_names: vocab.values().map(Vec::len).sum(),
    }
}

/// The contract of the fixture: the computed counts are the pinned
/// [`FIXTURE_COUNTS`], the only place that has them as numbers. The tests
/// of the other modules and crates use [`fixture_counts`]. A new fixture
/// command changes the numbers there.
#[test]
fn test_fixture_counts_are_pinned() {
    assert_eq!(computed_fixture_counts(), FIXTURE_COUNTS);
    assert_eq!(fixture_counts(), FIXTURE_COUNTS);
    // The counts agree with the build.
    let built = built();
    let counts = fixture_counts();
    assert_eq!(counts.records, built.commands.len());
    assert_eq!(counts.left_out, built.report.left_out.len());
    assert_eq!(counts.commands_in_profile, fixture_builder().len());
    assert_eq!(
        counts.loaded + counts.removed_by_number_check,
        counts.records
    );
}

#[test]
fn test_fixture_profile_builds_with_no_error_and_the_tested_format() {
    let built = built();
    eprintln!(
        "fixture: {} commands, {} records, {} left out",
        built.report.commands_in_profile,
        built.report.records,
        built.report.left_out.len()
    );
    assert_eq!(built.report.format, FormatNote::Tested);
    assert!(
        built.report.records >= MIN_RECORDS,
        "{}",
        built.report.records
    );
    assert_eq!(built.report.commands_in_profile, fixture_builder().len());
    assert_eq!(
        built.report.commands_in_profile,
        built.report.records + built.report.left_out.len()
    );
    assert_eq!(built.commands, fixture_commands());
    assert_eq!(
        built.profile_vars,
        HashMap::from([("HarbourQuery".to_string(), HARBOUR_QUERY.to_string())])
    );
}

#[test]
fn test_fixture_has_one_or_more_commands_of_each_kind() {
    let built = built();
    let c = &built.commands;
    let any = |f: &dyn Fn(&Command) -> bool| c.iter().any(f);
    assert!(any(&|c| c.command_type == CommandType::Prefix));
    assert!(any(&|c| c.command_type == CommandType::Suffix));
    for class in [
        CommandClass::Fixed,
        CommandClass::DynamicName,
        CommandClass::DynamicValue,
    ] {
        assert!(any(&|c| c.class == class), "{class:?}");
    }
    assert!(any(&|c| !c.number_vars.is_empty()));
    assert!(any(&|c| !c.constants.is_empty()));
    assert!(any(&|c| c
        .phrase
        .as_deref()
        .is_some_and(|p| p.contains("{TXT:EDCP_"))));
    assert!(any(&|c| c
        .phrase
        .as_deref()
        .is_some_and(|p| p.contains("{TXT:System"))));
    assert!(any(&|c| c
        .phrase
        .as_deref()
        .is_some_and(|p| p.contains("{TXT:Group"))));
    assert!(any(&|c| c.token_template.contains("{CMD}")));
    assert!(any(&|c| c.token_template.ends_with("\r\n")));
    for class in [
        LeftOutClass::NoWrite,
        LeftOutClass::MultiWrite,
        LeftOutClass::Orphan,
        LeftOutClass::Maintenance,
        LeftOutClass::NotUnderstood,
    ] {
        assert!(
            built.report.left_out.iter().any(|l| l.class == class),
            "{class:?}"
        );
    }
}

#[test]
fn test_fixture_folder_loads_with_the_runtime_grammar_and_the_vocabulary() {
    let f = folder();
    // Only the command whose variable the INI does not have keeps it.
    let unresolved: Vec<_> = f
        .commands
        .iter()
        .filter_map(|c| c.phrase.as_deref())
        .filter(|p| p.contains("{TXT:EDCP_"))
        .collect();
    assert_eq!(unresolved, ["{TXT:EDCP_GATECHIME}"]);
    // The profile variable is in the phrases.
    assert!(f.commands.iter().all(|c| !c
        .phrase
        .as_deref()
        .unwrap_or("")
        .contains("{TXT:HarbourQuery}")));
    assert_eq!(f.vocab["System1"], ["Pellmar", "Pelmar", "Pellmar Quay"]);
    // The group files load under their own names, not as the slots
    // `Group<n>` of the phrases.
    assert_eq!(f.vocab["BookmarkGrp1"], ["Marta Quillon"]);
    assert!(f.vocab["BookmarkGrp3"].is_empty());
    assert!(!f.vocab.contains_key("Group1"));
    assert!(f.vocab["System11"].is_empty());
    assert_eq!(f.vocab["System12"], ["Tarn Ness"]);

    let p = pipeline();
    for (text, expected) in [
        ("sail to pellmar", "HarbourAt:Pellmar:sail to"),
        ("steer to pellmar quay", "HarbourAt:Pellmar:sail to"),
        ("sail to the old mill", "HarbourAt:Dunmore:sail to"),
        ("open tide log", "OpenTideLogPanel"),
        ("show the harbour log panel", "OpenTideLogPanel"),
        ("seek rock salt", "SetCargo:Salt"),
        ("dock sell rate for rope", "SellDock:Rope"),
        ("take me back", "SailHomePort"),
        ("mooring 7", "SelectMooring:7"),
    ] {
        assert_eq!(token(&p, text).as_deref(), Some(expected), "{text}");
    }
    // The grammar of this number command has a minus sign outside its
    // ranges: the command leaves the list at the load.
    assert_eq!(token(&p, "wick lamp level 4"), None);
    let m = hit(&p, "tide at pellmar");
    assert!(m.token.starts_with("HarbourInfo:Pellmar:"), "{m:?}");
}

// ---------------------------------------------------------------------
// The builder and the folder
// ---------------------------------------------------------------------

#[test]
fn test_profile_builder_gives_the_same_bytes_each_time() {
    assert_eq!(fixture_builder().build(), fixture_builder().build());
    assert_eq!(fixture_profile(), fixture_builder().build());
    let mut a = ProfileBuilder::new();
    a.category("Lamp room")
        .fixed("light the test lamp", "Lamp:Test:On");
    let mut b = ProfileBuilder::new();
    b.category("Lamp room")
        .fixed("light the test lamp", "Lamp:Test:On");
    assert_eq!(a.build(), b.build());
    assert_eq!(a.len(), 1);
    assert!(ProfileBuilder::new().is_empty());
}

/// Each command has its own id: an execute action by id finds one target.
#[test]
fn test_profile_builder_gives_each_command_its_own_id() {
    let raw = super::super::record::walk(&fixture_builder().inflated()).unwrap();
    let ids: HashSet<[u8; 16]> = raw.commands.iter().map(|c| c.id).collect();
    assert_eq!(ids.len(), raw.commands.len());
}

#[test]
fn test_write_fixture_edcopilot_writes_only_inside_the_folder() {
    let outer = tempfile::tempdir().unwrap();
    let dir = outer.path().join("EDCoPilot");
    write_fixture_edcopilot(&dir);
    fn files(dir: &Path, out: &mut BTreeSet<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                files(&path, out);
            } else {
                out.insert(path);
            }
        }
    }
    let mut all = BTreeSet::new();
    files(outer.path(), &mut all);
    let mut expected: BTreeSet<PathBuf> = VOCABULARY
        .iter()
        .map(|(name, _)| dir.join("Dynamic var files").join(name))
        .collect();
    expected.insert(dir.join("EDCoPilot-Profile.vap"));
    expected.insert(dir.join("working").join("EDCoPilot.VoiceCommands.ini"));
    assert_eq!(all, expected);
    // A second write into the same folder works and gives the same bytes.
    write_fixture_edcopilot(&dir);
    assert_eq!(
        std::fs::read(dir.join("EDCoPilot-Profile.vap")).unwrap(),
        fixture_profile()
    );
}

// ---------------------------------------------------------------------
// The property groups
// ---------------------------------------------------------------------

/// A multi-word phrase, a one-word phrase, two phrases that differ by one
/// word, a prefix phrase, alternatives with optional parts, several
/// phrases in one command, and a tie.
#[test]
fn test_fixture_fixed_phrase_shapes() {
    let c = fixture_commands();
    assert_eq!(
        by_phrase(&c, "help tide log").token_template,
        "Guide:Panel:TideLog"
    );
    assert_eq!(by_phrase(&c, "belay").class, CommandClass::Fixed);
    by_phrase(&c, "raise the blue flag");
    by_phrase(&c, "raise the red flag");
    by_phrase(&c, "ring bell");
    by_phrase(&c, "ring bell twice");
    by_phrase(&c, "[raise; hoist] the storm [cone; signal] [now;]");
    by_phrase(&c, "toll the chapel bell; strike the chapel bell");
    assert_eq!(
        by_phrase(&c, "sound the noon gun").token_template,
        "Gun:Noon\r\n"
    );

    let p = pipeline();
    let m = hit(&p, "belay");
    assert_eq!(m.token, "Belay");
    assert!(!m.inexact_single_required_word);
    assert!(hit(&p, "belai").inexact_single_required_word);
    assert_eq!(
        token(&p, "raise the red flag").as_deref(),
        Some("Flag:Red:Up")
    );
    assert_eq!(token(&p, "ring bell").as_deref(), Some("Bell:Ring"));
    assert_eq!(
        token(&p, "ring bell twice").as_deref(),
        Some("Bell:RingTwice")
    );
    assert_eq!(
        token(&p, "hoist the storm signal").as_deref(),
        Some("Signal:StormCone")
    );
    assert_eq!(
        token(&p, "strike the chapel bell").as_deref(),
        Some("Bell:Chapel")
    );
    // A two-word fuzzy phrase passes the threshold.
    let m = hit(&p, "help tide logg");
    assert_eq!(m.token, "Guide:Panel:TideLog");
    assert!(m.confidence < 1.0);
    // Two commands match the same text with different tokens.
    assert_eq!(token(&p, "close the shutter"), None);
    assert_eq!(
        token(&p, "shut the shutter").as_deref(),
        Some("LampShutter:Close")
    );
}

/// Number variables: ranges, a scaled range, thousands, a number at the
/// end and in the middle, and a value of two numbers.
#[test]
fn test_fixture_number_commands_have_ranges() {
    let f = folder();
    let numbers: Vec<&Command> = f
        .commands
        .iter()
        .filter(|c| !c.number_vars.is_empty())
        .collect();
    assert!(numbers.len() >= 15, "{}", numbers.len());
    for c in &numbers {
        let p = c.phrase.as_deref().unwrap();
        assert!(p.contains(".."), "no range in {p:?}");
        assert_eq!(c.class, CommandClass::DynamicValue, "{p:?}");
    }
    let p = pipeline();
    for (text, expected) in [
        ("lamp brightness fifty five", "Prefs:LampBrightness=\"55\""),
        ("fog horn volume fifty five", "Prefs:HornVolume=\"55\""),
        ("fog horn volume fifty-five", "Prefs:HornVolume=\"55\""),
        ("mist horn volume one hundred", "Prefs:HornVolume=\"100\""),
        (
            "show buoys within 12 thousand yards",
            "ShowBuoysWithin=12000",
        ),
        (
            "show buoys within twelve thousand yards",
            "ShowBuoysWithin=12000",
        ),
        ("show buoys within 12,000 yards", "ShowBuoysWithin=12000"),
        ("hoist pennant seven", "Pennant:7"),
        ("ring the bell three times", "Bell:Times=3"),
        ("net reminder 40", "SetNetReminder:40"),
        ("set crate count to 20", "SetCrateCount:20"),
        ("add lamp oil to the store by 5 barrels", "AddLampOil:5"),
        ("pick mooring three four", "PickMooringVoice:34"),
        ("set the context to mooring 2 3", "MooringContext:23"),
        ("mooring zero five", "SelectMooring:05"),
    ] {
        assert_eq!(token(&p, text).as_deref(), Some(expected), "{text}");
    }
    // A range takes no leading zero, and a digit run is no value.
    for text in [
        "fog horn volume zero five",
        "fog horn volume five five",
        "net reminder zero five zero",
    ] {
        assert_eq!(token(&p, text), None, "{text}");
    }
    // The number in the middle of a {CMD} token.
    assert_eq!(
        token(&p, "nearest berth size 3 steam crane").as_deref(),
        Some("Berth:Steam Crane~nearest berth size 3 steam crane")
    );
}

/// System slots with the slot parse, the vocabulary names, and the group
/// slots with first-name alternates.
#[test]
fn test_fixture_slot_commands_and_vocabulary() {
    let c = fixture_commands();
    for n in 1..=SYSTEM_SLOTS {
        let sail = format!("{{TXT:EDCP_SAILTO}} {{TXT:System{n}}}");
        assert_eq!(by_phrase(&c, &sail).class, CommandClass::DynamicName);
        by_phrase(&c, &format!("{{TXT:HarbourQuery}} {{TXT:System{n}}}"));
    }
    for n in 1..=GROUP_SLOTS {
        by_phrase(&c, &format!("hail {{TXT:Group{n}}}"));
    }
    let f = folder();
    let names: Vec<&String> = (1..=SYSTEM_SLOTS)
        .filter_map(|n| f.vocab.get(&format!("System{n}")))
        .filter_map(|v| v.first())
        .collect();
    assert!(names.len() >= 10, "{names:?}");
    // Names with digits, a number word, and an English word.
    assert!(names.iter().any(|n| n.as_str() == "KTL 3081"));
    assert!(names.iter().any(|n| n.as_str() == "17 Skerry"));
    assert!(names.iter().any(|n| n.as_str() == "Seven Gulls"));
    assert!(names.iter().any(|n| n.as_str() == "Morrow"));
    // Two names that sound alike.
    assert!(strsim::normalized_levenshtein("vellin", "vellan") >= 0.8);

    let p = pipeline();
    for (text, expected) in [
        (
            "sail to ktl three zero eight one",
            "HarbourAt:KTL 3081:sail to",
        ),
        ("sail to one seven skerry", "HarbourAt:17 Skerry:sail to"),
        ("sail to reef 2207", "HarbourAt:Reef 2207:sail to"),
        ("sail to seven gulls", "HarbourAt:Seven Gulls:sail to"),
        ("sail to morrow", "HarbourAt:Morrow:sail to"),
        ("sail to vellin", "HarbourAt:Vellin:sail to"),
        ("sail to vellan", "HarbourAt:Vellan:sail to"),
        (
            "sail to brackmoor sownd",
            "HarbourAt:Brackmoor Sound:sail to",
        ),
    ] {
        assert_eq!(token(&p, text).as_deref(), Some(expected), "{text}");
    }
    // A one-word name with an error is flagged.
    assert!(hit(&p, "sail to pellmer").inexact_single_required_word);
    // A group slot has no names: the group commands do not match. This is
    // the behaviour on a real installation (see `fixture`).
    assert_eq!(token(&p, "hail marta quillon"), None);
}

/// Constants with a fixed name slot from the INI.
#[test]
fn test_fixture_constants_get_fixed_name_slots() {
    let c = fixture_commands();
    let constants: Vec<&Command> = c.iter().filter(|c| !c.constants.is_empty()).collect();
    assert!(constants.len() >= 6);
    assert!(constants
        .iter()
        .all(|c| c.constants.contains_key("SystemName")));
    let f = folder();
    let fixed: Vec<&Command> = f
        .commands
        .iter()
        .filter(|c| !c.fixed_vocab.is_empty())
        .collect();
    assert!(fixed.len() >= 5, "{}", fixed.len());
    let mill = fixed
        .iter()
        .find_map(|c| c.fixed_vocab.get("Fixed:OLDMILL:Dunmore"))
        .unwrap();
    assert_eq!(mill, &["The Old Mill", "Dunmore"]);
    let absent = fixed
        .iter()
        .find_map(|c| c.fixed_vocab.get("Fixed:LOSTKEEPER:Graymouth"))
        .unwrap();
    assert_eq!(absent, &["Graymouth"]);
    // A name with a range has no spoken forms: the phrase keeps it.
    assert!(f
        .commands
        .iter()
        .any(|c| c.phrase.as_deref() == Some("[sail to; steer to] peak [1..3]")));

    let p = pipeline();
    for (text, expected) in [
        ("sail to the old mill", "HarbourAt:Dunmore:sail to"),
        ("sail to dunmore", "HarbourAt:Dunmore:sail to"),
        ("sail to tam the ferryman", "HarbourAt:Brackwater:sail to"),
        ("sail to halvard crane", "HarbourAt:Vennick:sail to"),
        ("sail to graymouth", "HarbourAt:Graymouth:sail to"),
        // A constant system that is also a slot name gives one token.
        ("sail to ollin", "HarbourAt:Ollin:sail to"),
        ("sail to rose quay", "HarbourAt:Ollin:sail to"),
    ] {
        assert_eq!(token(&p, text).as_deref(), Some(expected), "{text}");
    }
    let m = hit(&p, "tide at rose quay");
    assert!(m.token.starts_with("HarbourInfo:Ollin:"), "{m:?}");
    assert!(hit(&p, "sail to dunmor").inexact_single_required_word);
}

/// The composite group: prefixes that choose a mode, suffixes that name
/// an item, with fixed and runtime-grammar phrases.
#[test]
fn test_fixture_composite_group() {
    let c = fixture_commands();
    let prefixes: Vec<&Command> = c
        .iter()
        .filter(|c| c.command_type == CommandType::Prefix)
        .collect();
    let suffixes: Vec<&Command> = c
        .iter()
        .filter(|c| c.command_type == CommandType::Suffix)
        .collect();
    assert_eq!(prefixes.len(), 5);
    assert!(suffixes.len() >= 12, "{}", suffixes.len());
    assert!(prefixes
        .iter()
        .chain(&suffixes)
        .all(|c| c.composite_group.as_deref() == Some(MARKET_GROUP)));
    assert!(prefixes.iter().all(|c| c.sets.contains_key("offer_scope")));
    assert!(suffixes.iter().all(|c| c.prefix_rules.len() == 4));
    assert!(suffixes
        .iter()
        .any(|c| c.phrase.as_deref().unwrap().contains("{TXT:EDCP_RATE_")));

    let p = pipeline();
    for (text, expected) in [
        ("dock sell cork", "SellDock:Cork"),
        ("give me the dock sell rate for cork", "SellDock:Cork"),
        ("dearest sell bid cork", "SellTop:Cork"),
        ("cheapest purchase bid tar", "BuyTop:Tar"),
        ("dearest sale bid rope", "SellTop:Rope"),
        ("nearest harbour to buy twine", "BuyNear:Twine"),
        ("nearest sell canvas", "SellNear:Canvas"),
        ("ways of buying wax", "BuyWays:Wax"),
        ("dock buy fish nets", "BuyDock:Fish Nets"),
        ("dock sell lamp oil", "SellDock:Lamp Oil"),
        ("dock sell rock salt", "SellDock:Salt"),
    ] {
        assert_eq!(token(&p, text).as_deref(), Some(expected), "{text}");
    }
    // A prefix without a buy or sell word leaves the side open.
    assert_eq!(token(&p, "cargo report on cork"), None);
    // The prefix grammar has no "quay to".
    assert_eq!(token(&p, "nearest quay to sell cork"), None);
    // A full command has the phrase of a prefix alone.
    assert_eq!(token(&p, "dock sell").as_deref(), Some("FindDockSell"));
    assert_eq!(token(&p, "dearest sell"), None);
    assert_eq!(token(&p, "rate for cork"), None);
    // A one-word item must pass alone: "cord" is 0.75 from "cork".
    assert_eq!(token(&p, "dock sell cord"), None);
    let m = hit(&p, "dock sell twines");
    assert!(m.inexact_single_required_word, "{m:?}");
    let m = hit(&p, "dock sell fish netz");
    assert_eq!(m.token, "SellDock:Fish Nets");
    assert!(!m.inexact_single_required_word);
    // The full command with one required word and optional words.
    assert_eq!(token(&p, "seek cork").as_deref(), Some("SetCargo:Cork"));
    assert_eq!(token(&p, "cork").as_deref(), Some("SetCargo:Cork"));
    assert!(hit(&p, "find corks").inexact_single_required_word);
}

/// Each left-out class, and three kinds of "not understood".
#[test]
fn test_fixture_left_out_classes_and_reasons() {
    let built = built();
    let reason = |phrase: &str| {
        let l = built
            .report
            .left_out
            .iter()
            .find(|l| l.phrase == phrase)
            .unwrap_or_else(|| panic!("{phrase:?} is not left out"));
        (l.class, l.reason.clone())
    };
    assert_eq!(reason("load harbour queries").0, LeftOutClass::NoWrite);
    assert_eq!(reason("ring all bells").0, LeftOutClass::MultiWrite);
    assert_eq!(reason("[for;] driftwood").0, LeftOutClass::Orphan);
    assert_eq!(
        reason("reset the lamp profile").0,
        LeftOutClass::Maintenance
    );
    assert_eq!(
        reason("rebuild the lamp profile").0,
        LeftOutClass::Maintenance
    );
    let (class, text) = reason("polish the lens");
    assert_eq!(class, LeftOutClass::NotUnderstood);
    assert!(text.contains("action type 99 (not known)"), "{text}");
    let (class, text) = reason("report the wind");
    assert_eq!(class, LeftOutClass::NotUnderstood);
    assert!(text.contains("does not model"), "{text}");
    let (class, text) = reason("test the lamp");
    assert_eq!(class, LeftOutClass::NotUnderstood);
    assert!(text.contains("code that is not reviewed"), "{text}");
    // A target that writes nothing is left out, and its caller stays.
    assert_eq!(reason("horn blast effect").0, LeftOutClass::NoWrite);
    assert_eq!(
        by_phrase(&built.commands, "sound the horn").token_template,
        "Horn:Blast"
    );
    // Nothing else is left out.
    assert_eq!(
        built.report.left_out.len(),
        fixture_counts().left_out,
        "{:?}",
        built.report.left_out
    );
    let p = pipeline();
    for text in [
        "ring all bells",
        "reset the lamp profile",
        "polish the lens",
        "test the lamp",
    ] {
        assert_eq!(token(&p, text), None, "{text}");
    }
}

/// The commands that do not work, for each reason of the Commands list.
#[test]
fn test_fixture_commands_that_do_not_work() {
    let f = folder();
    let needs_value: Vec<&str> = f
        .commands
        .iter()
        .filter(|c| {
            !c.is_composite_part()
                && c.class == CommandClass::DynamicValue
                && c.number_vars.is_empty()
        })
        .filter_map(|c| c.phrase.as_deref())
        .collect();
    assert_eq!(
        needs_value,
        [
            "set lamp colour",
            "lock the boathouse",
            "unlock the boathouse",
            "lock lantern"
        ]
    );
    let unresolved = f
        .commands
        .iter()
        .filter(|c| c.phrase.as_deref().unwrap_or("").contains("{TXT:EDCP_"))
        .count();
    assert_eq!(unresolved, 1);
    let parts = f.commands.iter().filter(|c| c.is_composite_part()).count();
    assert!(parts >= 17, "{parts}");
    // A needs-value command never becomes a log command through a
    // sound-alike: its text gives no match.
    let p = pipeline();
    assert_eq!(p.match_text("lock the boathouse"), None);
}

/// Six to eight categories with several commands each. No phrase is a
/// category name.
#[test]
fn test_fixture_categories() {
    let c = fixture_commands();
    let mut count: HashMap<&str, usize> = HashMap::new();
    for cmd in &c {
        *count.entry(cmd.category.as_deref().unwrap()).or_default() += 1;
    }
    assert_eq!(count.len(), CATEGORIES.len());
    assert!((6..=8).contains(&count.len()));
    for name in CATEGORIES {
        assert!(count[name] >= 5, "{name}: {}", count[name]);
    }
    for cmd in &c {
        let phrase = cmd.phrase.as_deref().unwrap();
        assert!(!CATEGORIES.contains(&phrase), "{phrase}");
    }
}

/// The phrases for the sound-alike, synonym, and filler safety tests.
#[test]
fn test_fixture_safety_phrases() {
    let p = pipeline();
    // "lock" -> "log" only when the text as heard matches nothing.
    for (text, expected) in [
        ("open tide lock", "OpenTideLogPanel"),
        ("help tide lock", "Guide:Panel:TideLog"),
        ("lay kurs for home port", "PlotCourse:HomePort"),
        ("net reminder free", "SetNetReminder:3"),
        ("cheapest by bid cork", "BuyTop:Cork"),
        ("higher water mark", "TideMark:Highest"),
        ("dearest sail bid cork", "SellTop:Cork"),
    ] {
        let m = hit(&p, text);
        assert_eq!(m.token, expected, "{text}");
        assert!(m.via_sound_alike, "{text}");
    }
    // A table word that is a phrase word: the text matches as heard.
    for (text, expected) in [
        ("lock current ledger", "LedgerLock:Current"),
        ("lamp cell spares", "Spares:Lamp Cell"),
        ("sort nets by length", "Prefs:SortNets=\"2\""),
        ("higher tide alerts", "Alerts:HigherTide"),
        ("pilot cutter mark two", "Vessel:CutterMk2"),
        ("add lamp oil to the store by five barrels", "AddLampOil:5"),
        ("steer the pilot boat", "PilotBoat:Steer"),
    ] {
        let m = hit(&p, text);
        assert_eq!(m.token, expected, "{text}");
        assert!(!m.via_sound_alike, "{text}");
    }
    for text in ["lock", "lock the tide", "unlock tide log", "by", "cell"] {
        assert_eq!(token(&p, text), None, "{text}");
    }
    // Filler words.
    let m = hit(&p, "uh help tide log please");
    assert_eq!(m.token, "Guide:Panel:TideLog");
    assert!(m.via_filler);
    let m = hit(&p, "head for the lighthouse");
    assert!(!m.via_filler);
    let m = hit(&p, "nearest berth for a size 3 steam crane");
    assert!(m.via_filler, "{m:?}");
    assert!(m.token.starts_with("Berth:Steam Crane~"), "{m:?}");
    let m = hit(&p, "sail to tam the ferryman");
    assert!(!m.via_filler);
    for text in [
        "lamp off the",
        "dump the nets for",
        "net reminder for 40",
        "set crate count to twenty for",
        "sail to the pellmar a",
    ] {
        assert_eq!(token(&p, text), None, "{text}");
    }
}

/// A working command with a text of a command that needs a value: the text
/// gives no command, and `shadowed_phrases` names the working command. Its
/// other text works.
#[test]
fn test_fixture_working_phrase_that_a_value_command_shadows() {
    let f = folder();
    let start = std::time::Instant::now();
    let found = crate::shadowed_phrases(&f.commands);
    eprintln!(
        "shadowed_phrases on {} fixture commands: {:?}",
        f.commands.len(),
        start.elapsed()
    );
    assert_eq!(
        found,
        [crate::ShadowedPhrase {
            phrase: "[paint the lamp; set lamp colour]".into(),
            text: "set lamp colour".into(),
        }]
    );
    let p = pipeline();
    assert_eq!(
        p.match_result("set lamp colour"),
        crate::MatchResult::Blocked(crate::BlockReason::NoToken)
    );
    assert_eq!(token(&p, "paint the lamp").as_deref(), Some("Lamp:Paint"));
}

// ---------------------------------------------------------------------
// Blocking phrases of the left-out commands

/// The left-out commands that VoiceAttack recognises by voice give a
/// blocking phrase each: each left-out command except the orphan suffix.
/// After the load, the command that the number check takes out blocks too.
#[test]
fn test_fixture_blocking_phrases() {
    let built = built();
    let phrases: Vec<&str> = built.blocking.iter().map(|b| b.phrase.as_str()).collect();
    let spoken: Vec<&str> = built
        .report
        .left_out
        .iter()
        .filter(|l| l.class != LeftOutClass::Orphan)
        .map(|l| l.phrase.as_str())
        .collect();
    assert_eq!(phrases, spoken);
    assert!(built
        .blocking
        .iter()
        .all(|b| b.command_type == CommandType::Full));
    let f = folder();
    assert_eq!(f.blocking.len(), fixture_counts().blocking_loaded);
    assert!(f
        .blocking
        .iter()
        .any(|b| b.class == LeftOutClass::NotUnderstood && b.phrase.contains("wick")));
}

/// A left-out command one fuzzy word from a working command: its text
/// blocks, the working text still matches. Without the blocking phrases,
/// the text of the left-out command gives the token of the other command.
#[test]
fn test_fixture_left_out_text_blocks_a_fuzzy_match() {
    let f = folder();
    let with = Matcher::new(f.commands.clone(), f.vocab.clone(), AliasMap::new(), 0.8)
        .with_blocking(&f.blocking);
    assert_eq!(
        with.match_result("test the lamp"),
        MatchResult::Blocked(BlockReason::NoToken)
    );
    assert_eq!(
        with.match_text("test the lamps")
            .map(|m| m.token)
            .as_deref(),
        Some("Lamp:TestAll")
    );
    let without = Matcher::new(f.commands, f.vocab, AliasMap::new(), 0.8);
    assert_eq!(
        without
            .match_text("test the lamp")
            .map(|m| m.token)
            .as_deref(),
        Some("Lamp:TestAll")
    );
}

/// A left-out command whose sound-alike variant is a working phrase: the
/// pipeline blocks the text as heard, so the variant does not decide. The
/// working phrase still matches. Without the blocking phrases, the variant
/// gives the token of the other command.
#[test]
fn test_fixture_left_out_text_blocks_a_sound_alike_variant() {
    let p = pipeline();
    assert_eq!(
        p.match_result("lock the wind speed"),
        MatchResult::Blocked(BlockReason::NoToken)
    );
    assert_eq!(
        token(&p, "log the wind speed").as_deref(),
        Some("LogWind:Speed")
    );
    let without = pipeline_with(false);
    assert_eq!(
        token(&without, "lock the wind speed").as_deref(),
        Some("LogWind:Speed")
    );
}
