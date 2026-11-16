//! Live probes: the matcher and the pipeline on the real EDCoPilot profile.
//!
//! These tests are ignored. They read the EDCoPilot folder from
//! `VOICEBRIDGE_EDCOPILOT_DIR` and write nothing. The repository holds no
//! EDCoPilot text. Thus the probe data (the texts and the expected tokens)
//! is in a local file that git does not track: the path is in
//! `VOICEBRIDGE_LIVE_PROBES`, the default is `private/live_probes.tsv`.
//! A relative path starts at the workspace folder. Without the file at the
//! default path, the probe test writes a note to stderr and passes. When
//! `VOICEBRIDGE_LIVE_PROBES` is set and the file cannot be read (also a
//! file that is not UTF-8), the test fails with the cause.
//!
//! ```text
//! VOICEBRIDGE_EDCOPILOT_DIR=C:\EDCoPilot cargo test -p edcopilot-voicebridge-core \
//!     --test live_probes -- --ignored --nocapture
//! ```
//!
//! # The format of the probe file
//!
//! The fields of a line are separated by a tab. An empty line and a line
//! that starts with `#` are not read.
//!
//! - `@section <name> <tables>` starts a section. `<tables>` is `none`, or
//!   a list with `,` of `sound_alikes`, `synonyms` and `fillers` (the
//!   default tables of `data/`). A new section has an empty vocabulary.
//! - `@vocab <slot> <names>` adds a vocabulary slot to the section, for
//!   example `System1`. The names are separated by `;`. The first name is
//!   the name of the slot.
//! - `@app <name>` starts a section for the reader of the `app` crate
//!   (`app/tests/live_probes.rs`). This reader does not read its lines.
//! - `<text> <expected> <checks>` is a probe. `<expected>` is the token,
//!   `-` for no match, `*` for a match with any token, or `?` for each
//!   result (also no match). The checks (optional, a list with `,`) apply
//!   to a match. With `*`, no match is a failure:
//!   - `synonym`, `sound_alike`, `filler`, `inexact`: the mark is set. A
//!     `!` before the name: the mark is not set. `sound_alike_or_synonym`:
//!     one of the two marks is set.
//!   - `matched`: the text gives a match.
//!   - `exact`: the confidence is 1. `fuzzy`: the confidence is 0.8 or
//!     more and less than 1. `conf=<value>`: the confidence is the value.
//!   - `contains=<text>`, `!contains=<text>`, `!token=<token>`: the token.
//!   - `same_as=<text>`: the token of the other text. `same_score_as=<text>`:
//!     the confidence and the mark `inexact` of the other text.
//!   - `without=<table>`: the probe runs on the pipeline without the
//!     table. `same_without=<table>`: the result (also no match) is equal
//!     on the pipeline without the table.
//!
//! The reader reads all lines before a probe runs. These errors fail the
//! test, with the line number: a byte order mark at the start of the file,
//! an empty expected field, a line that starts with `@` and is not
//! `@section`, `@vocab` or `@app` (for example a typing error in a section
//! line, also in an `@app` section), an unknown table, an unknown check
//! (also on a probe that gives no match), a `@vocab` line after a probe
//! of its section, a second `@vocab` line for the same slot in one
//! section, a section without a probe line, and a line with too many
//! fields (more than 3, or more than 2 for `@app`). A probe line in an
//! `@app` section is not counted here.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use edcopilot_voicebridge_core::profile::fixture::{
    fixture_profile, VOCABULARY, VOICE_COMMANDS_INI,
};
use edcopilot_voicebridge_core::profile::{build_map, BlockingPhrase};
use edcopilot_voicebridge_core::{
    load_fillers_with_problems, load_sound_alikes, load_synonyms_with_problems,
    removed_as_blocking, resolve_blocking, resolve_profile_vars, resolve_runtime_grammar_with,
    shadowed_phrases_with, AliasMap, BlockReason, Command, Match, MatchResult, Matcher, Pipeline,
    Writer,
};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn edcopilot_dir() -> PathBuf {
    PathBuf::from(
        std::env::var_os("VOICEBRIDGE_EDCOPILOT_DIR")
            .expect("set VOICEBRIDGE_EDCOPILOT_DIR to the EDCoPilot folder"),
    )
}

/// The real commands and blocking phrases as `load_commands` of the
/// program loads them.
struct RealLoad {
    commands: Vec<Command>,
    blocking: Vec<BlockingPhrase>,
    /// The blocking phrases that do not resolve or do not parse.
    skipped: usize,
}

/// The real commands as `load_commands` of the program loads them: the
/// build, the runtime grammar of the INI (with the number check), then the
/// profile variables. The blocking phrases of the build and of the
/// commands that the number check takes out, with the same grammar.
fn real_load(dir: &Path) -> RealLoad {
    let vap = std::fs::read(dir.join("EDCoPilot-Profile.vap")).unwrap();
    let built = build_map(&vap).unwrap();
    let ini = dir.join("working").join("EDCoPilot.VoiceCommands.ini");
    let out = resolve_runtime_grammar_with(built.commands, Some(&ini), &built.profile_vars);
    let mut blocking = built.blocking;
    blocking.extend(removed_as_blocking(&out.removed));
    let blocking = resolve_blocking(blocking, Some(&ini), &built.profile_vars);
    RealLoad {
        commands: resolve_profile_vars(out.commands, &built.profile_vars),
        blocking: blocking.phrases,
        skipped: blocking.skipped,
    }
}

/// The default word tables of a section.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
struct Tables {
    sound_alikes: bool,
    synonyms: bool,
    fillers: bool,
}

impl Tables {
    fn parse(text: &str) -> Result<Tables, String> {
        let mut t = Tables::default();
        if text.trim() == "none" {
            return Ok(t);
        }
        for name in text.split(',').map(str::trim) {
            *t.field(name)? = true;
        }
        Ok(t)
    }

    fn field(&mut self, name: &str) -> Result<&mut bool, String> {
        match name {
            "sound_alikes" => Ok(&mut self.sound_alikes),
            "synonyms" => Ok(&mut self.synonyms),
            "fillers" => Ok(&mut self.fillers),
            _ => Err(format!("unknown table {name:?}")),
        }
    }

    fn without(mut self, name: &str) -> Result<Tables, String> {
        *self.field(name)? = false;
        Ok(self)
    }
}

/// A section of the probe file: its tables, its vocabulary, and its
/// pipelines (made when a probe needs them).
struct Section {
    name: String,
    tables: Tables,
    vocab: HashMap<String, Vec<String>>,
    pipelines: BTreeMap<Tables, Pipeline>,
}

impl Section {
    fn pipeline(&mut self, load: &RealLoad, tables: Tables) -> &Pipeline {
        let vocab = &self.vocab;
        self.pipelines.entry(tables).or_insert_with(|| {
            let matcher = Matcher::new(load.commands.clone(), vocab.clone(), AliasMap::new(), 0.8)
                .with_blocking(&load.blocking);
            let writer = Writer::new(std::env::temp_dir().join("voicebridge-never-written.txt"));
            let mut p = Pipeline::new(matcher, writer);
            let data = root().join("data");
            if tables.sound_alikes {
                p = p
                    .with_sound_alikes(load_sound_alikes(&data.join("sound_alikes.toml")).unwrap());
            }
            if tables.synonyms {
                p = p.with_synonyms(load_synonyms_with_problems(&data.join("synonyms.toml")).0);
            }
            if tables.fillers {
                p = p.with_fillers(load_fillers_with_problems(&data.join("fillers.toml")).0);
            }
            p
        })
    }
}

/// The result of a probe in short form, for a failure message.
fn show(m: &Option<Match>) -> String {
    match m {
        None => "no match".to_string(),
        Some(m) => format!(
            "{:?} confidence {} inexact {} synonym {} sound-alike {} filler {}",
            m.token,
            m.confidence,
            m.inexact_single_required_word,
            m.via_synonym,
            m.via_sound_alike,
            m.via_filler
        ),
    }
}

/// Runs one probe. `Err` gives the reason of a failure.
fn run_probe(
    section: &mut Section,
    commands: &RealLoad,
    text: &str,
    expected: &str,
    checks: &[&str],
) -> Result<(), String> {
    let mut tables = section.tables;
    for c in checks {
        if let Some(name) = c.strip_prefix("without=") {
            tables = tables.without(name)?;
        }
    }
    let result = section.pipeline(commands, tables).match_text(text);
    match (expected, &result) {
        ("", _) => return Err("the expected field is empty".to_string()),
        ("-", None) | ("?", _) => {}
        ("*", Some(_)) => {}
        ("*", None) => return Err("expected a match, got no match".to_string()),
        ("-", Some(_)) => return Err(format!("expected no match, got {}", show(&result))),
        (token, Some(m)) if m.token == token => {}
        (token, _) => return Err(format!("expected {token:?}, got {}", show(&result))),
    }
    for check in checks {
        let check = check.trim();
        if check.is_empty() || check.starts_with("without=") {
            continue;
        }
        if let Some(name) = check.strip_prefix("same_without=") {
            let other = section.tables.without(name)?;
            let a = section
                .pipeline(commands, section.tables)
                .match_result(text);
            let b = section.pipeline(commands, other).match_result(text);
            if a != b {
                return Err(format!("{check}: {a:?} against {b:?}"));
            }
            continue;
        }
        if check == "matched" {
            if result.is_none() {
                return Err("expected a match".to_string());
            }
            continue;
        }
        // The other checks apply only to a match.
        let Some(m) = &result else { continue };
        let ok = match check {
            "synonym" => m.via_synonym,
            "!synonym" => !m.via_synonym,
            "sound_alike" => m.via_sound_alike,
            "!sound_alike" => !m.via_sound_alike,
            "filler" => m.via_filler,
            "!filler" => !m.via_filler,
            "inexact" => m.inexact_single_required_word,
            "!inexact" => !m.inexact_single_required_word,
            "sound_alike_or_synonym" => m.via_sound_alike || m.via_synonym,
            "exact" => m.confidence == 1.0,
            "fuzzy" => (0.8..1.0).contains(&m.confidence),
            _ => {
                let (key, value) = check
                    .split_once('=')
                    .ok_or_else(|| format!("unknown check {check:?}"))?;
                match key {
                    "conf" => {
                        let want: f32 = value
                            .parse()
                            .map_err(|_| format!("bad number in {check:?}"))?;
                        (m.confidence - want).abs() < 1e-6
                    }
                    "contains" => m.token.contains(value),
                    "!contains" => !m.token.contains(value),
                    "!token" => m.token != value,
                    "same_as" | "same_score_as" => {
                        let other = section.pipeline(commands, tables).match_text(value);
                        let Some(o) = other else {
                            return Err(format!("{check}: the other text gives no match"));
                        };
                        if key == "same_as" {
                            o.token == m.token
                        } else {
                            o.confidence == m.confidence
                                && o.inexact_single_required_word == m.inexact_single_required_word
                        }
                    }
                    _ => return Err(format!("unknown check {check:?}")),
                }
            }
        };
        if !ok {
            return Err(format!("check {check} failed: {}", show(&result)));
        }
    }
    Ok(())
}

/// The text of the probe file and its path. `None` when the default file
/// is absent (a note goes to stderr). It panics when the path of
/// `VOICEBRIDGE_LIVE_PROBES` cannot be read, when the file is not UTF-8,
/// and when the file starts with a byte order mark.
fn probe_file() -> Option<(PathBuf, String)> {
    // A relative path starts at the workspace folder.
    let explicit = std::env::var_os("VOICEBRIDGE_LIVE_PROBES").map(PathBuf::from);
    let path = root().join("..").join(
        explicit
            .clone()
            .unwrap_or_else(|| PathBuf::from("private/live_probes.tsv")),
    );
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if explicit.is_none() && e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!(
                "note: no probe file at {}, the live probes are skipped",
                path.display()
            );
            return None;
        }
        Err(e) => panic!("cannot read the probe file {}: {e}", path.display()),
    };
    assert!(
        !text.starts_with('\u{feff}'),
        "the probe file {} starts with a byte order mark",
        path.display()
    );
    Some((path, text))
}

/// One probe line of the file.
#[derive(Debug, Clone, PartialEq)]
struct Probe {
    /// The line number (from 1) and the text of the line.
    line: usize,
    raw: String,
    /// The index of the section in `ProbeFile::sections`.
    section: usize,
    text: String,
    expected: String,
    checks: Vec<String>,
}

/// The sections and the probes of the file, and its errors.
#[derive(Default)]
struct ProbeFile {
    sections: Vec<Section>,
    probes: Vec<Probe>,
    failures: Vec<String>,
}

/// Gives an error for a check that the reader does not know, before a
/// probe runs. Thus a typing error in a check is an error also on a
/// probe that gives no match.
fn check_name(check: &str) -> Result<(), String> {
    match check {
        "matched"
        | "synonym"
        | "!synonym"
        | "sound_alike"
        | "!sound_alike"
        | "filler"
        | "!filler"
        | "inexact"
        | "!inexact"
        | "sound_alike_or_synonym"
        | "exact"
        | "fuzzy" => Ok(()),
        _ => {
            let (key, value) = check
                .split_once('=')
                .ok_or_else(|| format!("unknown check {check:?}"))?;
            match key {
                "conf" => value
                    .parse::<f32>()
                    .map(|_| ())
                    .map_err(|_| format!("bad number in {check:?}")),
                "without" | "same_without" => Tables::default().without(value).map(|_| ()),
                "contains" | "!contains" | "!token" | "same_as" | "same_score_as"
                    if !value.is_empty() =>
                {
                    Ok(())
                }
                _ => Err(format!("unknown check {check:?}")),
            }
        }
    }
}

/// Reads the probe file text. The directives come first, also in an
/// `@app` section: a typing error in a section line after an `@app`
/// section is an error and does not hide the probes after it.
fn parse_probe_file(text: &str) -> ProbeFile {
    let mut out = ProbeFile::default();
    // True in a section of the `app` reader: its probes are not read here.
    let mut in_app = false;
    // The sections that have a probe: a vocabulary after a probe is an
    // error, because all probes of a section have the same vocabulary.
    let mut probed = HashSet::new();
    // The line number and the text of each section line, and the sections
    // that have a probe line (also a line with an error).
    let mut section_lines: Vec<(usize, String)> = Vec::new();
    let mut with_lines = HashSet::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        let at = |i: usize| fields.get(i).map(|f| f.trim()).unwrap_or("");
        let fail = |reason: String| format!("line {}: {line:?}: {reason}", n + 1);
        // A directive with too many fields is read all the same. A probe
        // with too many fields does not run.
        let max_fields = if at(0) == "@app" { 2 } else { 3 };
        let too_many = fields.len() > max_fields && !(in_app && !at(0).starts_with('@'));
        if too_many {
            out.failures.push(fail(format!(
                "{} fields, at most {max_fields}",
                fields.len()
            )));
        }
        match at(0) {
            "@app" => in_app = true,
            "@section" => {
                in_app = false;
                match Tables::parse(at(2)) {
                    Ok(tables) => {
                        section_lines.push((n + 1, line.to_string()));
                        out.sections.push(Section {
                            name: at(1).to_string(),
                            tables,
                            vocab: HashMap::new(),
                            pipelines: BTreeMap::new(),
                        });
                    }
                    Err(e) => out.failures.push(fail(e)),
                }
            }
            // The app reader gives the error for a vocabulary in its section.
            "@vocab" if in_app => {}
            "@vocab" => match out.sections.len().checked_sub(1) {
                Some(i) if probed.contains(&i) => out
                    .failures
                    .push(fail("a vocabulary after a probe of the section".into())),
                Some(i) if out.sections[i].vocab.contains_key(at(1)) => out
                    .failures
                    .push(fail(format!("a second vocabulary of the slot {:?}", at(1)))),
                Some(i) => {
                    let names = at(2).split(';').map(|n| n.trim().to_string()).collect();
                    out.sections[i].vocab.insert(at(1).to_string(), names);
                }
                None => out
                    .failures
                    .push(fail("a vocabulary before a section".into())),
            },
            other if other.starts_with('@') => out
                .failures
                .push(fail(format!("unknown directive {other:?}"))),
            _ if in_app => {}
            text => {
                let Some(section) = out.sections.len().checked_sub(1) else {
                    out.failures.push(fail("a probe before a section".into()));
                    continue;
                };
                with_lines.insert(section);
                if too_many {
                    continue;
                }
                let checks: Vec<String> = at(2)
                    .split(',')
                    .map(str::trim)
                    .filter(|c| !c.is_empty())
                    .map(str::to_string)
                    .collect();
                if at(1).is_empty() {
                    out.failures
                        .push(fail("the expected field is empty".into()));
                    continue;
                }
                if let Some(e) = checks.iter().find_map(|c| check_name(c).err()) {
                    out.failures.push(fail(e));
                    continue;
                }
                probed.insert(section);
                out.probes.push(Probe {
                    line: n + 1,
                    raw: line.to_string(),
                    section,
                    text: text.to_string(),
                    expected: at(1).to_string(),
                    checks,
                });
            }
        }
    }
    for (i, (n, line)) in section_lines.iter().enumerate() {
        if !with_lines.contains(&i) {
            out.failures
                .push(format!("line {n}: {line:?}: a section without a probe"));
        }
    }
    out
}

/// The probes of the private file on the real commands.
#[test]
#[ignore = "needs the EDCoPilot installation (VOICEBRIDGE_EDCOPILOT_DIR) and the private probe file"]
fn test_live_probes_from_the_private_file() {
    let Some((path, text)) = probe_file() else {
        return;
    };
    let commands = real_load(&edcopilot_dir());
    let ProbeFile {
        mut sections,
        probes,
        mut failures,
    } = parse_probe_file(&text);
    for probe in &probes {
        let section = &mut sections[probe.section];
        let checks: Vec<&str> = probe.checks.iter().map(String::as_str).collect();
        if let Err(e) = run_probe(section, &commands, &probe.text, &probe.expected, &checks) {
            failures.push(format!(
                "line {}: {:?}: [{}] {e}",
                probe.line, probe.raw, section.name
            ));
        }
    }
    println!(
        "live probes: {} probes in {} sections from {}, {} failures",
        probes.len(),
        sections.len(),
        path.display(),
        failures.len()
    );
    for f in &failures {
        println!("  FAIL {f}");
    }
    assert!(!probes.is_empty(), "the probe file has no probe");
    assert!(failures.is_empty(), "{} probes failed", failures.len());
}

#[test]
fn test_reader_reads_a_directive_also_after_an_app_section() {
    // A typing error in the section line after an `@app` section.
    let text = "@section\tharbour\tnone\n\
                ring the bell\tBellRing\n\
                @app\tbells\n\
                text\tring the bell\tBellRing\n\
                @sectoin\tdocks\tnone\n\
                open the gate\tGateOpen\n";
    let parsed = parse_probe_file(text);
    assert_eq!(parsed.failures.len(), 1, "{:?}", parsed.failures);
    assert!(parsed.failures[0].starts_with("line 5:"));
    assert!(parsed.failures[0].contains("unknown directive \"@sectoin\""));
    // With the correct line, the reader reads the probe of the new section.
    let parsed = parse_probe_file(&text.replace("@sectoin", "@section"));
    assert!(parsed.failures.is_empty(), "{:?}", parsed.failures);
    let texts: Vec<&str> = parsed.probes.iter().map(|p| p.text.as_str()).collect();
    assert_eq!(texts, ["ring the bell", "open the gate"]);
    assert_eq!(parsed.probes[1].section, 1);
}

#[test]
fn test_reader_reports_a_bad_check_before_the_probe_runs() {
    // A probe with `-` or `?` can give no match: then the checks do not
    // run. The reader reports the typing error all the same.
    for line in [
        "lower the flag\t-\tsynonm",
        "lower the flag\t?\t!sound_alike,exactly",
        "lower the flag\t-\twithout=filers",
        "lower the flag\t*\tconf=high",
        "lower the flag\t*\tcontains=",
    ] {
        let parsed = parse_probe_file(&format!("@section\tharbour\tnone\n{line}\n"));
        assert_eq!(parsed.failures.len(), 1, "{line:?}: {:?}", parsed.failures);
        assert!(parsed.probes.is_empty(), "{line:?}");
    }
    let good = "lower the flag\t-\tsynonym,!filler,without=fillers,same_without=synonyms,\
                conf=0.9,contains=Flag,!token=FlagUp,same_as=flag down,fuzzy,matched";
    let parsed = parse_probe_file(&format!("@section\tharbour\tsynonyms,fillers\n{good}\n"));
    assert!(parsed.failures.is_empty(), "{:?}", parsed.failures);
    assert_eq!(parsed.probes.len(), 1);
    assert_eq!(parsed.probes[0].checks.len(), 10);
}

#[test]
fn test_reader_reports_a_bad_section_or_vocabulary() {
    for (text, reason) in [
        ("@section\tharbour\tsound_alike\n", "unknown table"),
        (
            "@vocab\tPort1\tBrightwater\n",
            "a vocabulary before a section",
        ),
        ("ring the bell\tBellRing\n", "a probe before a section"),
        (
            "@section\tharbour\tnone\nring the bell\t\n",
            "the expected field is empty",
        ),
        (
            "@section\tharbour\tnone\nring\tBell\n@vocab\tPort1\tBrightwater\n",
            "a vocabulary after a probe",
        ),
    ] {
        let parsed = parse_probe_file(text);
        assert!(
            parsed.failures.iter().any(|f| f.contains(reason)),
            "{text:?}: {:?}",
            parsed.failures
        );
    }
    // The app reader reports a vocabulary in its section, this reader not.
    let parsed = parse_probe_file("@app\tbells\n@vocab\tPort1\tBrightwater\n");
    assert!(parsed.failures.is_empty(), "{:?}", parsed.failures);
    // The vocabulary of a section.
    let parsed =
        parse_probe_file("@section\tharbour\tnone\n@vocab\tPort1\tBrightwater;Bright Water\n");
    assert_eq!(
        parsed.sections[0].vocab["Port1"],
        ["Brightwater", "Bright Water"]
    );
}

#[test]
fn test_reader_reports_an_empty_section_a_second_vocabulary_and_extra_fields() {
    for (text, reason) in [
        // A section without a probe, before another section and at the end.
        (
            "@section\tharbour\tnone\n@section\tdocks\tnone\nring\tBell\n",
            "line 1: \"@section\\tharbour\\tnone\": a section without a probe",
        ),
        (
            "@section\tharbour\tnone\nring\tBell\n@section\tdocks\tnone\n",
            "line 3: \"@section\\tdocks\\tnone\": a section without a probe",
        ),
        // A section without a probe before an app section.
        (
            "@section\tharbour\tnone\n@app\tbells\n",
            "line 1: \"@section\\tharbour\\tnone\": a section without a probe",
        ),
        // The same slot two times in one section.
        (
            "@section\tharbour\tnone\n@vocab\tPort1\tBrightwater\n\
             @vocab\tPort1\tDeepwater\nring\tBell\n",
            "line 3: \"@vocab\\tPort1\\tDeepwater\": a second vocabulary of the slot \"Port1\"",
        ),
        // Extra fields.
        (
            "@section\tharbour\tnone\nring\tBell\tmatched\tx\n",
            "line 2: \"ring\\tBell\\tmatched\\tx\": 4 fields, at most 3",
        ),
        (
            "@section\tharbour\tnone\textra\nring\tBell\n",
            "line 1: \"@section\\tharbour\\tnone\\textra\": 4 fields, at most 3",
        ),
        (
            "@section\tharbour\tnone\n@vocab\tPort1\tBrightwater\textra\nring\tBell\n",
            "line 2: \"@vocab\\tPort1\\tBrightwater\\textra\": 4 fields, at most 3",
        ),
        (
            "@app\tbells\textra\n",
            "line 1: \"@app\\tbells\\textra\": 3 fields, at most 2",
        ),
    ] {
        let parsed = parse_probe_file(text);
        assert_eq!(parsed.failures, [reason], "{text:?}");
    }
    // The same slot in two sections, and a probe line in an app section
    // with more than 2 fields.
    let parsed = parse_probe_file(
        "@section\tharbour\tnone\n@vocab\tPort1\tBrightwater\nring\tBell\n\
         @section\tdocks\tnone\n@vocab\tPort1\tDeepwater\nring\tBell\tmatched\n\
         @app\tbells\ntext\tring the bell\tBellRing\n",
    );
    assert!(parsed.failures.is_empty(), "{:?}", parsed.failures);
    assert_eq!(parsed.probes.len(), 2);
}

/// No phrase, no token template, no `EDCP_` variable name, and no token
/// head word of the fixture is in the real profile (its records and its
/// left-out commands). No key, value, or section of the fixture INI is in
/// the real INI, and no vocabulary name of the fixture is in the real
/// vocabulary files.
#[test]
#[ignore = "needs the EDCoPilot installation (VOICEBRIDGE_EDCOPILOT_DIR)"]
fn test_fixture_holds_no_text_of_the_real_profile() {
    let vap = std::fs::read(edcopilot_dir().join("EDCoPilot-Profile.vap")).unwrap();
    let real = build_map(&vap).unwrap();
    let fixture = build_map(&fixture_profile()).unwrap();
    let norm = |p: &str| {
        p.to_lowercase()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    };
    let edcp = |p: &str| -> Vec<String> {
        p.match_indices("{TXT:EDCP_")
            .filter_map(|(at, _)| p[at..].find('}').map(|end| p[at..at + end].to_string()))
            .collect()
    };
    // The head of a token: the text before the first `:`, `=`, `~`, or
    // `{`. A head that is only a placeholder is empty.
    let head = |t: &str| -> String {
        t.split([':', '=', '~', '{'])
            .next()
            .unwrap_or("")
            .trim()
            .to_string()
    };
    let real_phrases: Vec<&str> = real
        .commands
        .iter()
        .filter_map(|c| c.phrase.as_deref())
        .chain(real.report.left_out.iter().map(|l| l.phrase.as_str()))
        .collect();
    let phrases: HashSet<String> = real_phrases.iter().map(|p| norm(p)).collect();
    let templates: HashSet<String> = real
        .commands
        .iter()
        .flat_map(|c| [c.token_template.clone(), c.effective_template()])
        .chain(real.report.left_out.iter().flat_map(|l| l.tokens.clone()))
        .filter(|t| !t.is_empty())
        .collect();
    let real_edcp: HashSet<String> = real_phrases.iter().flat_map(|p| edcp(p)).collect();
    let heads: HashSet<String> = templates.iter().map(|t| head(t)).collect();
    // The comparison is not empty.
    assert!(phrases.len() > 1000 && templates.len() > 1000);
    assert!(real_edcp.len() > 100 && heads.len() > 100);
    for c in &fixture.commands {
        if let Some(p) = c.phrase.as_deref() {
            assert!(!phrases.contains(&norm(p)), "phrase {p:?}");
            for v in edcp(p) {
                assert!(!real_edcp.contains(&v), "variable {v:?} in {p:?}");
            }
        }
        for t in [c.token_template.clone(), c.effective_template()] {
            if t.is_empty() {
                continue;
            }
            assert!(!templates.contains(&t), "template {t:?}");
            let h = head(&t);
            assert!(h.is_empty() || !heads.contains(&h), "head {h:?}");
        }
    }
    // The left-out commands of the fixture too.
    for l in &fixture.report.left_out {
        assert!(!phrases.contains(&norm(&l.phrase)), "phrase {:?}", l.phrase);
        for t in &l.tokens {
            assert!(!templates.contains(t), "template {t:?}");
        }
    }

    // The INI of the fixture: no key, description, grammar, or section
    // name of the real INI.
    let real_ini = std::fs::read(
        edcopilot_dir()
            .join("working")
            .join("EDCoPilot.VoiceCommands.ini"),
    )
    .unwrap();
    let real_ini = ini_parts(&String::from_utf8_lossy(&real_ini));
    let fixture_ini = ini_parts(VOICE_COMMANDS_INI);
    assert!(real_ini.keys.len() > 100 && real_ini.values.len() > 100);
    for key in &fixture_ini.keys {
        assert!(!real_ini.keys.contains(key), "INI key {key:?}");
    }
    for value in &fixture_ini.values {
        assert!(!real_ini.values.contains(value), "INI value {value:?}");
    }
    for section in &fixture_ini.sections {
        assert!(
            !real_ini.sections.contains(section),
            "INI section {section:?}"
        );
    }

    // The vocabulary of the fixture: no name of the real vocabulary files.
    let mut real_names = HashSet::new();
    for entry in std::fs::read_dir(edcopilot_dir().join("Dynamic var files")).unwrap() {
        let path = entry.unwrap().path();
        if let Ok(bytes) = std::fs::read(&path) {
            real_names.extend(vocabulary_names(&String::from_utf8_lossy(&bytes)));
        }
    }
    assert!(!real_names.is_empty());
    for (file, text) in VOCABULARY {
        for name in vocabulary_names(text) {
            assert!(!real_names.contains(&name), "name {name:?} of {file}");
        }
    }
}

/// The parts of an INI text, in lower case: the keys, the descriptions and
/// grammars (the fields after `=`), and the section names. Comment lines
/// (`\\`) are not read.
#[derive(Default)]
struct IniParts {
    keys: HashSet<String>,
    values: HashSet<String>,
    sections: HashSet<String>,
}

fn ini_parts(text: &str) -> IniParts {
    let mut parts = IniParts::default();
    for line in text.trim_start_matches('\u{feff}').lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('\\') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            parts.sections.insert(line.to_lowercase());
            continue;
        }
        let Some((key, rest)) = line.split_once('=') else {
            continue;
        };
        parts.keys.insert(key.trim().to_lowercase());
        for field in rest.split('|') {
            let field = field.trim().to_lowercase();
            if !field.is_empty() {
                parts.values.insert(field);
            }
        }
    }
    parts
}

/// The names of a vocabulary file in lower case: the entries of
/// `[a; b]`, or the whole text. "Not set" texts give no name.
fn vocabulary_names(text: &str) -> Vec<String> {
    let text = text.trim_start_matches('\u{feff}').trim();
    if text.is_empty() || text.starts_with("Not set") {
        return Vec::new();
    }
    let inner = text
        .strip_prefix('[')
        .and_then(|t| t.strip_suffix(']'))
        .unwrap_or(text);
    inner
        .split(';')
        .map(|n| n.trim().to_lowercase())
        .filter(|n| !n.is_empty())
        .collect()
}

/// The working commands of the real profile with a text that is also a
/// text of a command that the program cannot send (a command that needs a
/// value, or a left-out command: `shadowed_phrases_with`), and the time of
/// the search. It prints counts only. With `VOICEBRIDGE_SHADOW_OUT`, it
/// also writes the phrases and texts to that file, which must be a private
/// file that git does not track (for example in `private/`, which
/// `.gitignore` names). Each found text gives no command.
#[test]
#[ignore = "needs the EDCoPilot installation (VOICEBRIDGE_EDCOPILOT_DIR)"]
fn test_shadowed_phrases_on_the_real_profile() {
    let RealLoad {
        commands,
        blocking,
        skipped,
    } = real_load(&edcopilot_dir());
    let start = Instant::now();
    let found = shadowed_phrases_with(&commands, &blocking);
    let elapsed = start.elapsed();
    let value_only = shadowed_phrases_with(&commands, &[]).len();
    println!(
        "shadowed_phrases on {} real commands and {} blocking phrases ({skipped} skipped): {} \
         commands ({value_only} without the blocking phrases), {elapsed:?}",
        commands.len(),
        blocking.len(),
        found.len()
    );
    let m = Matcher::new(commands, HashMap::new(), AliasMap::new(), 0.8).with_blocking(&blocking);
    for s in &found {
        assert_eq!(
            m.match_result(&s.text),
            MatchResult::Blocked(BlockReason::NoToken),
            "a found text gives a result other than a block"
        );
    }
    if let Some(path) = std::env::var_os("VOICEBRIDGE_SHADOW_OUT") {
        let lines: Vec<String> = found
            .iter()
            .map(|s| format!("{}\t{}", s.text, s.phrase))
            .collect();
        std::fs::write(&path, lines.join("\n") + "\n").unwrap();
    }
}

/// The time of `Matcher::new` and of `match_text` on the real commands.
/// The texts are phrases of the profile without a group or a placeholder,
/// and one text that matches nothing.
#[test]
#[ignore = "needs the EDCoPilot installation (VOICEBRIDGE_EDCOPILOT_DIR)"]
fn test_timing_on_the_real_map() {
    let RealLoad {
        commands, blocking, ..
    } = real_load(&edcopilot_dir());
    let mut texts: Vec<String> = commands
        .iter()
        .filter_map(|c| c.phrase.as_deref())
        .filter(|p| !p.contains(['[', '{', ';']))
        .take(3)
        .map(str::to_string)
        .collect();
    texts.push("mmm gibberish words here".to_string());
    let records = commands.len();
    // Without and with the blocking phrases of the left-out commands.
    for with in [false, true] {
        let start = Instant::now();
        let m = Matcher::new(commands.clone(), HashMap::new(), AliasMap::new(), 0.8);
        let m = if with { m.with_blocking(&blocking) } else { m };
        let built = start.elapsed();
        let rounds = 5;
        let start = Instant::now();
        for _ in 0..rounds {
            for text in &texts {
                let _ = m.match_text(text);
            }
        }
        let per_match = start.elapsed() / (rounds * texts.len() as u32);
        let blocking = if with { blocking.len() } else { 0 };
        println!(
            "Matcher::new on {records} real records and {blocking} blocking phrases: {built:?}; \
             match_text average: {per_match:?}"
        );
    }
}
