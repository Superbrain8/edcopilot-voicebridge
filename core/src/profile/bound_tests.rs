//! The work of the builder stays bounded on crafted profiles at the limits
//! of the walk. Each test builds a profile of one shape that gave
//! quadratic work before, and requires a generous wall time. The names
//! are invented.

use std::time::{Duration, Instant};

use super::test_support::*;
use super::*;

/// The wall time of one build in these tests (debug build). The builds of
/// the shapes below took minutes before the bounds.
const LIMIT: Duration = Duration::from_secs(10);

/// Builds `commands` without filler commands and checks the wall time.
/// The result can be a map or a typed error.
fn timed(name: &str, commands: Vec<Vec<u8>>) -> Result<BuiltMap, BuildError> {
    let vap = deflate(&profile(commands));
    let started = Instant::now();
    let out = build_map(&vap);
    let took = started.elapsed();
    assert!(took < LIMIT, "{name}: {took:?}");
    out
}

fn id_of(k: usize) -> [u8; 16] {
    let mut id = [0u8; 16];
    id[..8].copy_from_slice(&(k as u64 + 1).to_le_bytes());
    id
}

/// Each caller runs another target by name after the write. The builder
/// finds at most [`allow::MAX_NAME_TARGETS`] names: the callers of the
/// other names are left out.
#[test]
fn test_many_execute_by_name_targets_finish_in_time() {
    let n = 8_000;
    let mut cmds: Vec<Vec<u8>> = (0..n)
        .map(|k| {
            command(
                &format!("caller {k}"),
                vec![
                    write(&format!("T{k}")),
                    exec_by_name(&format!("target {k}")),
                ],
                0,
                None,
            )
        })
        .collect();
    cmds.extend((0..n).map(|k| command(&format!("target {k}"), vec![sound()], 0, None)));
    cmds.extend(filler(200));
    let b = timed("execute by name", cmds).unwrap();
    assert_eq!(b.commands.len(), allow::MAX_NAME_TARGETS + 200);
}

/// Each caller runs its own root, and each root runs one hub. The hub
/// runs many leaves with many actions. The target check of the hub runs
/// once, not once for each root.
#[test]
fn test_many_callers_with_distinct_roots_of_one_hub_finish_in_time() {
    let (leaves, roots) = (1_000, 9_000);
    let sounds = 60;
    let mut cmds = vec![command_with_id(
        id_of(0),
        "hub",
        (0..leaves).map(|k| exec_by_id(id_of(1 + k))).collect(),
    )];
    cmds.extend(
        (0..leaves)
            .map(|k| command_with_id(id_of(1 + k), &format!("leaf {k}"), vec![sound(); sounds])),
    );
    cmds.extend((0..roots).map(|k| {
        command_with_id(
            id_of(1 + leaves + k),
            &format!("root {k}"),
            vec![exec_by_id(id_of(0))],
        )
    }));
    cmds.extend((0..roots).map(|k| {
        command_with_id(
            id_of(1 + leaves + roots + k),
            &format!("caller {k}"),
            vec![write(&format!("T{k}")), exec_by_id(id_of(1 + leaves + k))],
        )
    }));
    let b = timed("callers of one hub", cmds).unwrap();
    assert_eq!(b.commands.len(), roots);
}

/// Many sets with a token in the variable name, each with its own type 77
/// writer. Above the limit of the name patterns, each name fits: the
/// request path is unknown, and the build stops with a typed error.
#[test]
fn test_many_sets_with_a_token_in_the_name_finish_in_time() {
    let n = 8_000;
    let cmds: Vec<Vec<u8>> = (0..n)
        .map(|k| {
            let mut f = action_fields(77);
            f[6] = str_field(&format!("C{k}"));
            command(
                &format!("side {k}"),
                vec![
                    set_text(&format!("Lamp{k}x{{TXT:C{k}}}"), "v"),
                    record(f),
                    write(&format!("S{k}")),
                ],
                0,
                None,
            )
        })
        .collect();
    let e = timed("token names", cmds).unwrap_err();
    assert!(
        matches!(
            e,
            BuildError::Totals(Totals::TooFewRecords { found: 0, .. })
        ),
        "{e:?}"
    );
}

/// Many distinct phrase variables and many actions: the lookup of the
/// profile variables uses one index of the writers, not a walk of all
/// actions for each variable.
#[test]
fn test_many_distinct_phrase_variables_finish_in_time() {
    let n = 19_000;
    let cmds: Vec<Vec<u8>> = (0..n)
        .map(|k| {
            let mut actions = vec![sound(); 4];
            actions.push(write(&format!("T{k}")));
            command(&format!("{{TXT:V{k}}} go"), actions, 0, None)
        })
        .collect();
    let b = timed("phrase variables", cmds).unwrap();
    assert_eq!(b.commands.len(), n);
    assert_eq!(b.report.unknown_phrase_vars, n);
}

/// A phrase longer than [`extract::MAX_PHRASE_CHARS`] is left out with a
/// reason. The search of the placeholders is linear.
#[test]
fn test_a_long_phrase_is_left_out_and_the_placeholder_search_is_linear() {
    let long = format!("{}}} now", "{".repeat(200_000));
    let at_limit = "a".repeat(extract::MAX_PHRASE_CHARS);
    let over = "b".repeat(extract::MAX_PHRASE_CHARS + 1);
    let mut cmds = vec![
        command(&long, vec![write("X")], 0, None),
        command(&at_limit, vec![write("Y")], 0, None),
        command(&over, vec![write("Z")], 0, None),
    ];
    cmds.extend(filler(200));
    let b = timed("long phrases", cmds).unwrap();
    let reason = "the phrase is longer than 4096 characters";
    for phrase in [&long, &over] {
        let l = b
            .report
            .left_out
            .iter()
            .find(|l| &l.phrase == phrase)
            .unwrap();
        assert_eq!(l.reason, reason);
    }
    assert!(b
        .commands
        .iter()
        .any(|c| c.phrase.as_deref() == Some(&at_limit)));
    assert!(b
        .blocking
        .iter()
        .all(|p| p.phrase.chars().count() <= extract::MAX_PHRASE_CHARS));
    let started = Instant::now();
    let text = format!("{}}}", "{".repeat(1_000_000));
    assert!(extract::placeholders(&text).is_empty());
    let text = format!("{}{{TXT:x}}", "{".repeat(1_000_000));
    assert_eq!(extract::placeholders(&text), [extract::Place::Txt("x")]);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
}

/// A token longer than [`extract::MAX_TOKEN_CHARS`] is left out with a
/// reason.
#[test]
fn test_a_long_token_is_left_out() {
    let long = format!("T{}", "{TXT:x}".repeat(1_000));
    let mut cmds = vec![command("ring the long bell", vec![write(&long)], 0, None)];
    cmds.extend(filler(200));
    let b = timed("long token", cmds).unwrap();
    let l = b
        .report
        .left_out
        .iter()
        .find(|l| l.phrase == "ring the long bell")
        .unwrap();
    assert_eq!(l.reason, "the request text is longer than 1024 characters");
}

/// The limits of the walk are near the size of a real profile (3,058
/// commands, 19,569 actions, 103 actions in one command).
#[test]
fn test_the_walk_limits_are_near_a_real_profile() {
    assert_eq!(record::LIMITS.commands, 20_000);
    assert_eq!(record::LIMITS.actions, 200_000);
    assert_eq!(record::LIMITS.actions_per_command, 2_000);
    let cmds = (0..20_001)
        .map(|k| command(&format!("c{k}"), vec![], 0, None))
        .collect();
    assert_eq!(
        build_map(&deflate(&profile(cmds))).unwrap_err(),
        BuildError::TooMany {
            what: "commands",
            limit: 20_000
        }
    );
}

/// Many action types with a new value in a shared field: the tally keeps
/// at most [`record::MAX_TALLY_TYPES`] types (the others count together),
/// and the report keeps at most [`report::MAX_REPORT_SETTINGS`] counts.
#[test]
fn test_the_setting_tally_and_the_report_stay_small() {
    let mut cmds: Vec<Vec<u8>> = (0..300)
        .map(|k| {
            command(
                &format!("odd {k}"),
                vec![
                    set_field(plain_action(1000 + k), 25, vec![1; 16]),
                    write(&format!("Odd:{k}")),
                ],
                0,
                None,
            )
        })
        .collect();
    cmds.extend(filler(6_000));
    let b = timed("many types", cmds).unwrap();
    assert_eq!(b.report.unknown_settings.len(), report::MAX_REPORT_SETTINGS);
    let p = record::walk(&profile(
        (0..300)
            .map(|k| command(&format!("odd {k}"), vec![plain_action(1000 + k)], 0, None))
            .collect(),
    ))
    .unwrap();
    assert_eq!(
        p.settings.actions_by_type.len(),
        record::MAX_TALLY_TYPES + 1
    );
    assert_eq!(
        p.settings.actions_by_type.get(&record::OTHER_TYPES),
        Some(&(300 - record::MAX_TALLY_TYPES))
    );
}
