//! Live checks of the profile builder on the EDCoPilot installation.
//!
//! The repository has no map that comes from EDCoPilot. Thus these tests
//! need only the EDCoPilot folder. They read the profile, and they write
//! nothing there. `docs/PROFILE_FORMAT.md` (section "The last recorded
//! numbers") gives the counts that the bands below come from.
//!
//! Run them with:
//!
//! ```text
//! VOICEBRIDGE_EDCOPILOT_DIR=C:\EDCoPilot cargo test -p edcopilot-voicebridge-core \
//!     --test profile_live -- --ignored --nocapture
//! ```
//!
//! The snapshot test also needs `VOICEBRIDGE_MAP_SNAPSHOT`: the path of a
//! private snapshot file. A relative path starts at the repository folder.
//! With `VOICEBRIDGE_MAP_SNAPSHOT_WRITE=1` it writes the snapshot of the
//! current build to that path in place of the compare. The snapshot holds
//! EDCoPilot text. Thus the write refuses a path in the repository that is
//! not under `private/`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use edcopilot_voicebridge_core::profile::{build_map, BuiltMap, LeftOutClass};
use edcopilot_voicebridge_core::{Command, CommandClass, CommandType};
use serde_json::json;

/// The band of the number of records in a sane build of a current
/// profile. The profile of 1.11.755 gives 2,486. A build that keeps
/// commands that it must leave out gives more.
const RECORDS: std::ops::RangeInclusive<usize> = 2_000..=2_700;

/// The band of the number of commands that the builder does not
/// understand. The profile of 1.11.755 gives 502 (503 after the load).
const NOT_UNDERSTOOD: std::ops::RangeInclusive<usize> = 300..=700;

/// The lowest count of each reason group of `not_understood` with a
/// recorded count of 5 or more: half of the recorded count, rounded up.
/// The profile of 1.11.755 gives 482, 8 and 7. A build that keeps the
/// commands of one group gives fewer.
const REASON_MINIMUMS: [(&str, usize); 3] = [
    (
        "the command sets a part of its token in a way that the program does not model",
        241,
    ),
    (
        "action type 16 (execute a command) after the write: the program cannot find the \
         command that it runs",
        4,
    ),
    (
        "an action before the write is switched off in the profile, the program cannot know \
         what VoiceAttack does",
        4,
    ),
];

/// The version of the content of the snapshot file.
const SNAPSHOT_SCHEMA: u64 = 1;

/// The build of the profile in `VOICEBRIDGE_EDCOPILOT_DIR`.
fn build_real_profile() -> (PathBuf, usize, BuiltMap) {
    let dir = std::env::var_os("VOICEBRIDGE_EDCOPILOT_DIR")
        .expect("set VOICEBRIDGE_EDCOPILOT_DIR to the EDCoPilot folder");
    let path = PathBuf::from(dir).join("EDCoPilot-Profile.vap");
    let vap = std::fs::read(&path).unwrap();
    let built = build_map(&vap).expect("the real profile builds");
    (path, vap.len(), built)
}

#[test]
#[ignore = "needs the EDCoPilot installation (VOICEBRIDGE_EDCOPILOT_DIR)"]
fn test_real_profile_builds_with_counts_in_the_recorded_band() {
    let (path, len, built) = build_real_profile();
    let report = &built.report;
    let not_understood = report.left_out_count(LeftOutClass::NotUnderstood);
    println!(
        "built from {} ({len} bytes, sha256 {}, VoiceAttack {}): {} records, {} left out, \
         {not_understood} not understood, {} profile variables",
        path.display(),
        report.source.sha256,
        report.source.voiceattack_version,
        built.commands.len(),
        report.left_out.len(),
        built.profile_vars.len()
    );
    println!("format: {}", report.format);
    // The blocking phrases: by class and by command type. Counts only.
    let mut blocking: BTreeMap<(String, String), usize> = BTreeMap::new();
    for b in &built.blocking {
        let kind = format!("{:?}", b.command_type).to_lowercase();
        *blocking
            .entry((b.class.as_str().to_string(), kind))
            .or_default() += 1;
    }
    println!(
        "blocking phrases: {} of {} left-out commands",
        built.blocking.len(),
        report.left_out.len()
    );
    for ((class, kind), n) in &blocking {
        println!("  {n:4}  {class} {kind}");
    }
    let reasons = report.class_reasons(LeftOutClass::NotUnderstood);
    let listing: String = reasons
        .iter()
        .map(|(reason, n, _)| format!("  {n:4}  {reason}\n"))
        .collect();
    print!("{listing}");
    assert_eq!(built.commands.len(), report.records);
    assert!(
        RECORDS.contains(&built.commands.len()),
        "{} records, outside {RECORDS:?}. Reasons of not_understood:\n{listing}",
        built.commands.len()
    );
    assert!(
        NOT_UNDERSTOOD.contains(&not_understood),
        "{not_understood} commands not understood, outside {NOT_UNDERSTOOD:?}. Reasons:\n{listing}"
    );
    for (reason, minimum) in REASON_MINIMUMS {
        let n = reasons
            .iter()
            .find(|(r, _, _)| r == reason)
            .map_or(0, |(_, n, _)| *n);
        assert!(
            n >= minimum,
            "{n} commands with the reason \"{reason}\", fewer than {minimum}. Reasons:\n{listing}"
        );
    }
}

/// One command as a JSON value. All fields that the build gives.
fn command_json(c: &Command) -> serde_json::Value {
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
    let rules: Vec<serde_json::Value> = c
        .prefix_rules
        .iter()
        .map(|r| json!({"contains": r.contains, "set": r.set}))
        .collect();
    json!({
        "phrase": c.phrase,
        "token_template": c.token_template,
        "class": class,
        "var_source": c.var_source,
        "category": c.category,
        "constants": c.constants,
        "number_vars": c.number_vars,
        "command_type": command_type,
        "composite_group": c.composite_group,
        "sets": c.sets,
        "prefix_rules": rules,
    })
}

/// The snapshot of a build: the commands, the profile variables, and the
/// left-out commands.
fn snapshot_of(built: &BuiltMap) -> serde_json::Value {
    let left_out: Vec<serde_json::Value> = built
        .report
        .left_out
        .iter()
        .map(|l| {
            json!({
                "phrase": l.phrase,
                "class": l.class.as_str(),
                "tokens": l.tokens,
                "reason": l.reason,
            })
        })
        .collect();
    json!({
        "schema": SNAPSHOT_SCHEMA,
        "sha256": built.report.source.sha256,
        "voiceattack_version": built.report.source.voiceattack_version,
        "commands": built.commands.iter().map(command_json).collect::<Vec<_>>(),
        "profile_vars": built.profile_vars,
        "left_out": left_out,
    })
}

/// A JSON value as one text with the keys of each object in sorted order.
/// The text does not depend on the key order of the `serde_json` map type
/// (a feature of another crate can change that order).
fn canonical(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => {
            let sorted: BTreeMap<&String, String> =
                map.iter().map(|(k, v)| (k, canonical(v))).collect();
            let items: Vec<String> = sorted
                .into_iter()
                .map(|(k, v)| format!("{}:{v}", serde_json::Value::String(k.clone())))
                .collect();
            format!("{{{}}}", items.join(","))
        }
        serde_json::Value::Array(items) => {
            let items: Vec<String> = items.iter().map(canonical).collect();
            format!("[{}]", items.join(","))
        }
        other => other.to_string(),
    }
}

/// The items of a JSON list as canonical texts with their counts.
fn multiset(value: &serde_json::Value) -> BTreeMap<String, usize> {
    let mut out = BTreeMap::new();
    for item in value.as_array().map(Vec::as_slice).unwrap_or_default() {
        *out.entry(canonical(item)).or_insert(0) += 1;
    }
    out
}

/// The repository folder.
fn repository() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

/// The full form of `path`: the full form of its folder (which must
/// exist), then the file name. A relative path starts at the repository.
fn full_path(path: &std::path::Path) -> Result<PathBuf, String> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        repository().join(path)
    };
    let name = path
        .file_name()
        .ok_or_else(|| format!("{} has no file name", path.display()))?;
    let folder = path.parent().unwrap_or(std::path::Path::new("."));
    let folder = folder
        .canonicalize()
        .map_err(|e| format!("the folder of {} does not open: {e}", path.display()))?;
    Ok(folder.join(name))
}

/// The snapshot write goes to `path` only when the path is outside the
/// repository, or under `private/` (git does not track that
/// folder). Else the write can put EDCoPilot text into a tracked file.
fn snapshot_write_allowed(path: &std::path::Path) -> Result<PathBuf, String> {
    let full = full_path(path)?;
    let repo = repository()
        .canonicalize()
        .map_err(|e| format!("the repository folder does not open: {e}"))?;
    let private = repo.join("private");
    let lower = |p: &std::path::Path| p.to_string_lossy().to_lowercase();
    let inside = |p: &std::path::Path, folder: &std::path::Path| {
        let (p, folder) = (lower(p), lower(folder));
        p.starts_with(&(folder + std::path::MAIN_SEPARATOR_STR))
    };
    if inside(&full, &repo) && !inside(&full, &private) {
        return Err(format!(
            "refused: {} is in the repository but not under private/",
            full.display()
        ));
    }
    Ok(full)
}

#[test]
fn test_the_snapshot_write_refuses_a_tracked_path() {
    let repo = repository();
    for refused in [
        repo.join("core").join("snapshot.json"),
        repo.join("snapshot.json"),
        repo.join("docs").join("snapshot.json"),
        repo.join("private").join("..").join("x.json"),
        PathBuf::from("core/snapshot.json"),
    ] {
        assert!(
            snapshot_write_allowed(&refused).is_err(),
            "{}",
            refused.display()
        );
    }
    let private = repo.join("private");
    if private.is_dir() {
        assert!(snapshot_write_allowed(&private.join("built_map.json")).is_ok());
    }
    let outside = std::env::temp_dir().join("voicebridge_snapshot.json");
    assert!(snapshot_write_allowed(&outside).is_ok());
}

#[test]
fn test_the_canonical_text_sorts_the_keys() {
    let a: serde_json::Value =
        serde_json::from_str(r#"{"b":1,"a":{"d":[1,{"z":0,"y":2}],"c":null}}"#).unwrap();
    assert_eq!(
        canonical(&a),
        r#"{"a":{"c":null,"d":[1,{"y":2,"z":0}]},"b":1}"#
    );
}

/// The items of `a` that are not in `b`, with counts.
fn minus(a: &BTreeMap<String, usize>, b: &BTreeMap<String, usize>) -> usize {
    a.iter()
        .map(|(k, n)| n.saturating_sub(b.get(k).copied().unwrap_or(0)))
        .sum()
}

/// Compares the build of the real profile with a private snapshot. The
/// test is a skip, with a note, when `VOICEBRIDGE_MAP_SNAPSHOT` is not set.
#[test]
#[ignore = "needs the EDCoPilot installation and a private snapshot (VOICEBRIDGE_MAP_SNAPSHOT)"]
fn test_real_profile_build_equals_the_private_snapshot() {
    let Some(file) = std::env::var_os("VOICEBRIDGE_MAP_SNAPSHOT") else {
        eprintln!("skipped: set VOICEBRIDGE_MAP_SNAPSHOT to the path of a private snapshot file");
        return;
    };
    // A relative path starts at the repository, not at the crate folder.
    let file = repository().join(PathBuf::from(file));
    let (_, _, built) = build_real_profile();
    let now = snapshot_of(&built);
    if std::env::var_os("VOICEBRIDGE_MAP_SNAPSHOT_WRITE").is_some_and(|v| v == "1") {
        let file = snapshot_write_allowed(&file).unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(&file, serde_json::to_vec_pretty(&now).unwrap()).unwrap();
        println!(
            "wrote the snapshot {} ({} commands)",
            file.display(),
            built.commands.len()
        );
        return;
    }
    let text = std::fs::read_to_string(&file).unwrap();
    let then: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(then["schema"], SNAPSHOT_SCHEMA, "another snapshot schema");
    println!(
        "snapshot of profile sha256 {} (VoiceAttack {}), build of sha256 {}",
        then["sha256"], then["voiceattack_version"], now["sha256"]
    );
    let mut equal = true;
    for part in ["commands", "left_out"] {
        let (old, new) = (multiset(&then[part]), multiset(&now[part]));
        let (only_old, only_new) = (minus(&old, &new), minus(&new, &old));
        println!(
            "{part}: snapshot {}, build {}, only in the snapshot {only_old}, only in the build \
             {only_new}",
            old.values().sum::<usize>(),
            new.values().sum::<usize>()
        );
        equal &= only_old == 0 && only_new == 0;
    }
    let vars = |v: &serde_json::Value| -> BTreeSet<(String, String)> {
        v.as_object()
            .map(|o| o.iter().map(|(k, v)| (k.clone(), v.to_string())).collect())
            .unwrap_or_default()
    };
    let same_vars = vars(&then["profile_vars"]) == vars(&now["profile_vars"]);
    println!("profile variables equal: {same_vars}");
    assert!(
        equal && same_vars,
        "the build differs from the snapshot {}",
        file.display()
    );
}
