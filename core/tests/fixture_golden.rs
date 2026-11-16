//! Golden test of the fixture build: each record, each left-out command,
//! each profile variable, the counts of the report, and each blocking
//! phrase of
//! `build_map(&fixture_profile())`, and the result of the load steps that
//! change the list (the runtime grammar of the fixture INI file and the
//! number check), with the blocking phrases after the load. Each item is
//! one canonical line.
//! The lines are sorted. The test compares them with the tracked file
//! `core/tests/golden/fixture_map.txt` (invented data only).
//!
//! A change of the builder that changes one field of one record makes this
//! test fail. When the change is correct, write the file again:
//!
//! ```text
//! VOICEBRIDGE_GOLDEN_WRITE=1 cargo test -p edcopilot-voicebridge-core --test fixture_golden
//! ```
//!
//! Then read the difference of the file in git before the commit.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use edcopilot_voicebridge_core::profile::fixture::{fixture_profile, write_fixture_edcopilot};
use edcopilot_voicebridge_core::profile::{build_map, BlockingPhrase, LeftOut};
use edcopilot_voicebridge_core::{
    removed_as_blocking, resolve_blocking, resolve_runtime_grammar_with, Command, CommandClass,
    CommandType, RemovedCommand,
};

/// The path of the golden file. The write goes to this path only.
fn golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
        .join("fixture_map.txt")
}

/// A text value in the quoted form of Rust: each special character is an
/// escape, so a value cannot break the line.
fn q(text: &str) -> String {
    format!("{text:?}")
}

fn opt(text: &Option<String>) -> String {
    text.as_deref().map_or_else(|| "none".to_string(), q)
}

/// A map in key order.
fn map(values: &HashMap<String, String>) -> String {
    let sorted: BTreeMap<&String, &String> = values.iter().collect();
    let items: Vec<String> = sorted
        .into_iter()
        .map(|(k, v)| format!("{}={}", q(k), q(v)))
        .collect();
    format!("{{{}}}", items.join(", "))
}

/// A list in its own order.
fn list(values: &[String]) -> String {
    let items: Vec<String> = values.iter().map(|v| q(v)).collect();
    format!("[{}]", items.join(", "))
}

/// One command with every field.
fn command_line(kind: &str, c: &Command) -> String {
    let class = match c.class {
        CommandClass::Fixed => "fixed",
        CommandClass::DynamicName => "dynamic_name",
        CommandClass::DynamicValue => "dynamic_value",
    };
    let command_type = match c.command_type {
        CommandType::Full => "full",
        CommandType::Prefix => "prefix",
        CommandType::Suffix => "suffix",
    };
    let rules: Vec<String> = c
        .prefix_rules
        .iter()
        .map(|r| format!("{{contains={} set={}}}", q(&r.contains), map(&r.set)))
        .collect();
    let fixed: BTreeMap<&String, &Vec<String>> = c.fixed_vocab.iter().collect();
    let fixed: Vec<String> = fixed
        .into_iter()
        .map(|(k, v)| format!("{}={}", q(k), list(v)))
        .collect();
    format!(
        "{kind} phrase={} token={} class={class} var_source={} category={} constants={} \
         number_vars={} type={command_type} group={} sets={} prefix_rules=[{}] fixed_vocab={{{}}}",
        opt(&c.phrase),
        q(&c.token_template),
        opt(&c.var_source),
        opt(&c.category),
        map(&c.constants),
        list(&c.number_vars),
        opt(&c.composite_group),
        map(&c.sets),
        rules.join(", "),
        fixed.join(", ")
    )
}

fn left_out_line(l: &LeftOut) -> String {
    format!(
        "build.left_out phrase={} class={} tokens={} reason={}",
        q(&l.phrase),
        l.class.as_str(),
        list(&l.tokens),
        q(&l.reason)
    )
}

fn blocking_line(kind: &str, b: &BlockingPhrase) -> String {
    let command_type = match b.command_type {
        CommandType::Full => "full",
        CommandType::Prefix => "prefix",
        CommandType::Suffix => "suffix",
    };
    format!(
        "{kind} phrase={} class={} type={command_type} group={}",
        q(&b.phrase),
        b.class.as_str(),
        opt(&b.composite_group)
    )
}

fn removed_line(r: &RemovedCommand) -> String {
    format!(
        "load.removed phrase={} token={} reason={}",
        q(&r.phrase),
        q(&r.token_template),
        q(&r.reason)
    )
}

/// The canonical lines of the fixture build and of the load, sorted.
fn golden_lines() -> Vec<String> {
    let built = build_map(&fixture_profile()).expect("the fixture profile builds");
    let mut lines: Vec<String> = Vec::new();
    lines.extend(
        built
            .commands
            .iter()
            .map(|c| command_line("build.record", c)),
    );
    lines.extend(built.report.left_out.iter().map(left_out_line));
    lines.extend(
        built
            .profile_vars
            .iter()
            .map(|(k, v)| format!("build.profile_var name={} grammar={}", q(k), q(v))),
    );
    lines.extend(
        built
            .blocking
            .iter()
            .map(|b| blocking_line("build.blocking", b)),
    );
    let r = &built.report;
    lines.push(format!(
        "build.report records={} left_out={} unknown_phrase_vars={} cannot_fire={} \
         unknown_settings={}",
        r.records,
        r.left_out.len(),
        r.unknown_phrase_vars,
        r.cannot_fire,
        r.unknown_settings.len()
    ));
    let dir = tempfile::tempdir().unwrap();
    write_fixture_edcopilot(dir.path());
    let ini = dir
        .path()
        .join("working")
        .join("EDCoPilot.VoiceCommands.ini");
    let resolved = resolve_runtime_grammar_with(built.commands, Some(&ini), &built.profile_vars);
    lines.extend(
        resolved
            .commands
            .iter()
            .map(|c| command_line("load.command", c)),
    );
    lines.extend(resolved.removed.iter().map(removed_line));
    // The blocking phrases after the load: those of the build and those of
    // the removed commands, with the runtime grammar.
    let mut blocking = built.blocking.clone();
    blocking.extend(removed_as_blocking(&resolved.removed));
    let blocking = resolve_blocking(blocking, Some(&ini), &built.profile_vars);
    lines.extend(
        blocking
            .phrases
            .iter()
            .map(|b| blocking_line("load.blocking", b)),
    );
    lines.push(format!("load.blocking_skipped count={}", blocking.skipped));
    lines.sort();
    lines
}

/// The first lines where `expected` and `actual` differ, for the failure
/// text: the first differing position, then the lines that only one side
/// has (at most 10 of each).
fn difference(expected: &[String], actual: &[String]) -> String {
    let first = expected
        .iter()
        .zip(actual)
        .position(|(e, a)| e != a)
        .unwrap_or(expected.len().min(actual.len()));
    let mut out = format!(
        "the golden file has {} lines, the build gives {}; the first difference is at line {}\n",
        expected.len(),
        actual.len(),
        first + 1
    );
    out += &format!(
        "  file:  {}\n",
        expected.get(first).map_or("(end)", String::as_str)
    );
    out += &format!(
        "  build: {}\n",
        actual.get(first).map_or("(end)", String::as_str)
    );
    let only = |a: &[String], b: &[String]| -> Vec<String> {
        a.iter()
            .filter(|l| !b.contains(l))
            .take(10)
            .cloned()
            .collect()
    };
    for line in only(expected, actual) {
        out += &format!("  only in the file:  {line}\n");
    }
    for line in only(actual, expected) {
        out += &format!("  only in the build: {line}\n");
    }
    out
}

#[test]
fn test_the_fixture_build_and_load_equal_the_golden_file() {
    let actual = golden_lines();
    let path = golden_path();
    if std::env::var_os("VOICEBRIDGE_GOLDEN_WRITE").is_some_and(|v| v == "1") {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, actual.join("\n") + "\n").unwrap();
        println!("wrote {} ({} lines)", path.display(), actual.len());
        return;
    }
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "{} does not open ({e}). Write it with VOICEBRIDGE_GOLDEN_WRITE=1",
            path.display()
        )
    });
    // Git can give the file CRLF line ends.
    let expected: Vec<String> = text
        .lines()
        .map(|l| l.trim_end_matches('\r').to_string())
        .filter(|l| !l.is_empty())
        .collect();
    assert!(
        expected == actual,
        "the fixture build differs from {}\n{}",
        path.display(),
        difference(&expected, &actual)
    );
}

#[test]
fn test_the_difference_names_the_first_differing_lines() {
    let file = vec!["a".to_string(), "b".to_string(), "c".to_string()];
    let build = vec!["a".to_string(), "x".to_string(), "c".to_string()];
    let text = difference(&file, &build);
    assert!(text.contains("at line 2"), "{text}");
    assert!(text.contains("only in the file:  b"), "{text}");
    assert!(text.contains("only in the build: x"), "{text}");
}
