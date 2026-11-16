//! Tests of the builder through `build_map`, on synthetic profiles with
//! invented phrases and tokens.

use super::test_support::*;
use super::*;

fn left<'a>(built: &'a BuiltMap, phrase: &str) -> Option<&'a LeftOut> {
    built.report.left_out.iter().find(|l| l.phrase == phrase)
}

fn mapped(built: &BuiltMap, phrase: &str) -> bool {
    built
        .commands
        .iter()
        .any(|c| c.phrase.as_deref() == Some(phrase))
}

/// The reason when the command is left out as not understood.
fn not_understood(built: &BuiltMap, phrase: &str) -> String {
    assert!(!mapped(built, phrase), "{phrase:?} is in the map");
    let l = left(built, phrase).unwrap_or_else(|| panic!("{phrase:?} is not left out"));
    assert_eq!(l.class, LeftOutClass::NotUnderstood, "{l:?}");
    l.reason.clone()
}

/// The code hash (hex) that the walk gives for one inline code action.
fn code_hash_hex(action: &[u8]) -> String {
    let p = record::walk(&profile(vec![command("x", vec![action.to_vec()], 0, None)])).unwrap();
    p.commands[0].actions[0]
        .code_hash
        .unwrap()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn built(commands: Vec<Vec<u8>>) -> BuiltMap {
    build_map(&vap(commands)).unwrap()
}

fn flag_command(actions: Vec<Vec<u8>>) -> Vec<u8> {
    command("raise the blue flag", actions, 0, None)
}

#[test]
fn test_build_map_from_deflated_bytes() {
    let b = built(vec![
        command("open the hatch", vec![sound(), write("OpenHatch")], 0, None),
        command("stop", vec![sound()], 0, None),
    ]);
    assert!(mapped(&b, "open the hatch"));
    assert_eq!(left(&b, "stop").unwrap().class, LeftOutClass::NoWrite);
    let r = &b.report;
    assert_eq!(r.commands_in_profile, 202);
    assert_eq!(r.records, 201);
    assert_eq!(r.source.actions, 203);
    assert_eq!(r.source.request_writes, 201);
    assert_eq!(r.source.sha256.len(), 64);
    assert_eq!(r.source.voiceattack_version, "1.16");
    assert_eq!(r.format, FormatNote::Tested);
}

#[test]
fn test_more_fields_at_the_end_give_a_note_and_the_same_map() {
    let cmds = || {
        vec![
            flag_command(vec![sound(), write("BlueFlag:Up")]),
            command(
                "lower the blue flag",
                vec![condition("x", "y", 6), say("ok"), end_condition()],
                0,
                None,
            ),
        ]
    };
    let tested = built(cmds());
    let newer = with_extra_fields(3, || build_map(&vap(cmds()))).unwrap();
    assert_eq!(tested.commands, newer.commands);
    assert_eq!(tested.report.left_out, newer.report.left_out);
    match &newer.report.format {
        FormatNote::NewerThanTested(counts) => {
            assert_eq!(counts.profile, 92);
            assert_eq!(counts.command, [85].into());
            assert_eq!(counts.action, [35].into());
        }
        other => panic!("expected a newer format, got {other:?}"),
    }
    let text = newer.report.format.to_string();
    assert!(text.contains("newer"), "{text}");
}

#[test]
fn test_fewer_fields_than_a_rule_reads_is_an_error() {
    // An action with the header count 29: field 30 is absent.
    let mut f = action_fields(14);
    f.truncate(30);
    let err = build_map(&vap(vec![flag_command(vec![
        record(f),
        write("BlueFlag:Up"),
    ])]));
    assert!(
        matches!(
            err,
            Err(BuildError::FieldCount {
                kind: RecordKind::Action,
                minimum: 30,
                found: 29,
                ..
            })
        ),
        "{err:?}"
    );
    // Fewer fields than the tested format, but enough for the rules: an
    // older format, whose field meanings were never checked.
    let mut f = action_fields(14);
    f.truncate(32);
    let err = build_map(&vap(vec![flag_command(vec![
        record(f),
        write("BlueFlag:Up"),
    ])]));
    assert_eq!(err.unwrap_err(), BuildError::OlderFormat);
    // A field that a rule reads with another encoding stops the build.
    let bad = set_field(set_text("Colour", "blue"), 14, vec![9, 0, 0, 0, b'x']);
    let err = build_map(&vap(vec![flag_command(vec![bad, write("BlueFlag:Up")])]));
    assert!(
        matches!(err, Err(BuildError::BadField { field: 14, .. })),
        "{err:?}"
    );
    // A field of the length signature with another length.
    let bad = set_field(set_text("Colour", "blue"), 13, vec![0, 0]);
    let err = build_map(&vap(vec![flag_command(vec![bad, write("BlueFlag:Up")])]));
    assert!(
        matches!(err, Err(BuildError::Layout { field: 13, .. })),
        "{err:?}"
    );
}

#[test]
fn test_unknown_action_type_before_the_write_leaves_the_command_out() {
    let b = built(vec![
        flag_command(vec![plain_action(99), write("BlueFlag:Up")]),
        command("raise the red flag", vec![write("RedFlag:Up")], 0, None),
    ]);
    let reason = not_understood(&b, "raise the blue flag");
    assert_eq!(reason, "action type 99 (not known) before the write");
    assert!(mapped(&b, "raise the red flag"));
}

#[test]
fn test_unknown_action_type_after_the_write_leaves_the_command_out() {
    let b = built(vec![flag_command(vec![
        write("BlueFlag:Up"),
        plain_action(99),
    ])]);
    assert_eq!(
        not_understood(&b, "raise the blue flag"),
        "action type 99 (not known) after the write"
    );
    // Known types that the table does not allow after the write.
    for (kind, name) in [
        (30, "begin a loop"),
        (31, "end a loop"),
        (75, "exit a loop"),
        (57, "stop all commands"),
        (22, "plugin function"),
        (71, "inline code"),
        (74, "wait for a spoken response"),
        (69, "purpose not known"),
    ] {
        let b = built(vec![flag_command(vec![
            write("BlueFlag:Up"),
            plain_action(kind),
        ])]);
        assert_eq!(
            not_understood(&b, "raise the blue flag"),
            format!("action type {kind} ({name}) after the write")
        );
    }
    // A write to another file after the write.
    let b = built(vec![flag_command(vec![
        write("BlueFlag:Up"),
        write_other_file("log"),
    ])]);
    assert_eq!(
        not_understood(&b, "raise the blue flag"),
        "action type 44 (write to a file) after the write"
    );
}

#[test]
fn test_condition_and_set_actions_after_the_write_keep_the_command() {
    let b = built(vec![flag_command(vec![
        write("Flag:{TXT:Colour}"),
        condition("Colour", "blue", 6),
        set_text("Colour", "red"),
        else_branch(),
        set_text_from_file("Colour", "colour.txt"),
        end_condition(),
        condition("Wind", "", 9),
        plain_action(COND_ELSE_IF_KIND),
        end_condition(),
        plain_action(18),
        plain_action(24),
        plain_action(36),
        plain_action(37),
        plain_action(38),
        plain_action(77),
    ])]);
    assert!(mapped(&b, "raise the blue flag"), "{:?}", b.report.left_out);
    // A set after the write gives no constant: the token then has a
    // placeholder that no path fills, and the matcher sends nothing.
    let b = built(vec![flag_command(vec![
        write("Flag:{TXT:SystemName}"),
        set_text("SystemName", "Far Away"),
    ])]);
    assert!(record_of_phrase(&b, "raise the blue flag")
        .constants
        .is_empty());
    // A second request write inside a condition after the write.
    let b = built(vec![flag_command(vec![
        write("BlueFlag:Up"),
        condition("Wind", "", 9),
        write("BlueFlag:Down"),
        end_condition(),
    ])]);
    assert_eq!(
        left(&b, "raise the blue flag").unwrap().class,
        LeftOutClass::MultiWrite
    );
}

const COND_ELSE_IF_KIND: i32 = 63;

#[test]
fn test_reviewed_code_in_a_target_keeps_the_command() {
    let code = "invented code: set one variable";
    let reviewed = [code_hash_hex(&inline_code(code))];
    let reviewed: Vec<&str> = reviewed.iter().map(String::as_str).collect();
    // The hash covers the assembly list (field 6) too.
    let other_assemblies = set_field(inline_code(code), 6, str_field("Other.dll"));
    let b = build_map_reviewed(
        &vap(vec![
            command_with_id([1; 16], "wave the flag", vec![other_assemblies]),
            flag_command(vec![write("BlueFlag:Up"), exec_by_id([1; 16])]),
        ]),
        &reviewed,
    )
    .unwrap();
    assert!(!mapped(&b, "raise the blue flag"));
    let cmds = |code: &str| {
        vec![
            command_with_id([1; 16], "wave the flag", vec![inline_code(code)]),
            flag_command(vec![write("BlueFlag:Up"), exec_by_id([1; 16])]),
        ]
    };
    let b = build_map_reviewed(&vap(cmds(code)), &reviewed).unwrap();
    assert!(mapped(&b, "raise the blue flag"), "{:?}", b.report.left_out);
    // A one-byte change of the code.
    let b = build_map_reviewed(&vap(cmds("invented code: set one variablf")), &reviewed).unwrap();
    assert_eq!(
        not_understood(&b, "raise the blue flag"),
        "action type 16 (execute a command) after the write: the command that it runs has \
         code that is not reviewed"
    );
    // The table of the program does not hold the invented code.
    let b = build_map(&vap(cmds(code))).unwrap();
    assert!(!mapped(&b, "raise the blue flag"));
    // Reviewed code directly in a command of the map stays not allowed.
    let b = build_map_reviewed(
        &vap(vec![flag_command(vec![
            write("BlueFlag:Up"),
            inline_code(code),
        ])]),
        &reviewed,
    )
    .unwrap();
    assert_eq!(
        not_understood(&b, "raise the blue flag"),
        "action type 71 (inline code) after the write"
    );
    // A plugin in a target stays not allowed.
    let b = build_map_reviewed(
        &vap(vec![
            command_with_id([1; 16], "wave the flag", vec![plain_action(22)]),
            flag_command(vec![write("BlueFlag:Up"), exec_by_id([1; 16])]),
        ]),
        &reviewed,
    )
    .unwrap();
    assert!(!mapped(&b, "raise the blue flag"));
}

#[test]
fn test_report_groups_left_out_commands_by_reason() {
    let b = built(vec![
        command("one", vec![sound()], 0, None),
        command("two", vec![sound()], 0, None),
        command("three", vec![plain_action(99), write("Three")], 0, None),
    ]);
    assert_eq!(
        b.report.reasons(),
        [
            (
                "the command writes no request token".to_string(),
                2,
                "one".to_string()
            ),
            (
                "action type 99 (not known) before the write".to_string(),
                1,
                "three".to_string()
            ),
        ]
    );
}

#[test]
fn test_inert_types_before_and_after_the_write_keep_the_command() {
    let inert = || [2, 13, 14, 23, 28, 34, 24].map(plain_action).to_vec();
    let mut actions = inert();
    actions.push(write("BlueFlag:Up"));
    actions.extend(inert());
    actions.extend([3, 4, 15, 50, 51, 70, 76].map(plain_action));
    let b = built(vec![flag_command(actions)]);
    assert!(mapped(&b, "raise the blue flag"), "{:?}", b.report.left_out);
    // The clipboard type before the write only when the token has no {CLIP}.
    let b = built(vec![flag_command(vec![
        plain_action(24),
        write("BlueFlag:{CLIP}"),
    ])]);
    assert_eq!(
        not_understood(&b, "raise the blue flag"),
        "action type 24 (set the clipboard) before the write"
    );
}

#[test]
fn test_variable_and_condition_actions_before_the_write_keep_the_command() {
    let b = built(vec![flag_command(vec![
        plain_action(18),
        plain_action(36),
        plain_action(37),
        plain_action(38),
        set_text("Colour", "blue"),
        set_text_from_file("Mood", "mood.txt"),
        condition("Colour", "blue", 6),
        say("blue"),
        else_branch(),
        set_text("Wind", "calm"),
        end_condition(),
        write("BlueFlag:Up"),
    ])]);
    assert!(mapped(&b, "raise the blue flag"), "{:?}", b.report.left_out);
}

#[test]
fn test_set_text_from_a_file_of_a_token_variable_leaves_the_command_out() {
    let reason = "the command sets a part of its token in a way that the program does not model";
    for set in [
        set_text_from_file("Colour", "colour.txt"),
        set_text_copy("Colour", "Other"),
        set_text_replace("Colour", "blue", "b"),
    ] {
        let b = built(vec![flag_command(vec![set, write("Flag:{TXT:Colour}")])]);
        assert_eq!(not_understood(&b, "raise the blue flag"), reason);
    }
    // A plain set of a token variable that is not a constant or a number
    // variable.
    let b = built(vec![flag_command(vec![
        set_text("Colour", "blue"),
        set_text("Colour", "red"),
        write("Flag:{TXT:Colour}"),
    ])]);
    assert_eq!(not_understood(&b, "raise the blue flag"), reason);
    // A set of a token variable inside a condition block.
    let b = built(vec![flag_command(vec![
        condition("x", "y", 6),
        set_text("SystemName", "Far Away"),
        end_condition(),
        write("Flag:{TXT:SystemName}"),
    ])]);
    assert_eq!(not_understood(&b, "raise the blue flag"), reason);
    // The modelled constant is kept.
    let b = built(vec![flag_command(vec![
        set_text("SystemName", "Far Away"),
        write("Flag:{TXT:SystemName}"),
    ])]);
    assert!(mapped(&b, "raise the blue flag"), "{:?}", b.report.left_out);
}

#[test]
fn test_number_variable_must_follow_the_model() {
    let good = || {
        vec![
            set_text("LastCMD", "{CMD}"),
            set_text("Height", "{TXTNUM:LastCMD}"),
            write("Flag:{TXT:Height}"),
        ]
    };
    let b = built(vec![command("raise the flag [1..9]", good(), 0, None)]);
    assert!(
        mapped(&b, "raise the flag [1..9]"),
        "{:?}",
        b.report.left_out
    );
    // The number comes before the set of LastCMD.
    let mut acts = good();
    acts.swap(0, 1);
    let b = built(vec![command("raise the flag [1..9]", acts, 0, None)]);
    assert_eq!(
        not_understood(&b, "raise the flag [1..9]"),
        "the number variable is not set as the program models it"
    );
}

#[test]
fn test_write_in_a_condition_outside_the_suffix_shape_leaves_the_command_out() {
    let prefix = command(
        "fly the [blue; red]",
        vec![set_text("Size", "Big:")],
        1,
        Some("flags"),
    );
    let good = || {
        vec![
            sound(),
            condition("{PREFIX}", "blue", 6),
            set_text("Colour", "Blue"),
            end_condition(),
            condition("{PREFIX}", "red", 6),
            set_text("Colour", "Red"),
            end_condition(),
            condition("Colour", "", 9),
            say("which flag"),
            else_branch(),
            write("{TXT:Size}{TXT:Colour}Flag"),
            end_condition(),
        ]
    };
    let b = built(vec![
        prefix.clone(),
        command("[now;] flag", good(), 2, Some("flags")),
    ]);
    assert!(mapped(&b, "[now;] flag"), "{:?}", b.report.left_out);
    assert!(mapped(&b, "fly the [blue; red]"));

    // The write inside a {PREFIX} block.
    let mut acts = good();
    acts.insert(3, write("BlueFlag"));
    acts.remove(11);
    let b = built(vec![
        prefix.clone(),
        command("[now;] flag", acts, 2, Some("flags")),
    ]);
    assert!(not_understood(&b, "[now;] flag").starts_with("the suffix"));

    // A second action in the else branch.
    let mut acts = good();
    acts.insert(11, sound());
    let b = built(vec![
        prefix.clone(),
        command("[now;] flag", acts, 2, Some("flags")),
    ]);
    assert!(not_understood(&b, "[now;] flag").starts_with("the suffix"));

    // A {PREFIX} block that compares with a second variable.
    let mut acts = good();
    acts[1] = set_field(acts[1].clone(), 21, int_field(1));
    let b = built(vec![prefix, command("[now;] flag", acts, 2, Some("flags"))]);
    assert!(not_understood(&b, "[now;] flag").starts_with("the suffix"));
}

#[test]
fn test_execute_command_before_the_write_leaves_the_command_out() {
    let target = command_with_id([1; 16], "wave the flag", vec![sound()]);
    let b = built(vec![
        target,
        flag_command(vec![exec_by_id([1; 16]), write("BlueFlag:Up")]),
    ]);
    assert_eq!(
        not_understood(&b, "raise the blue flag"),
        "action type 16 (execute a command) before the write"
    );
}

#[test]
fn test_execute_command_after_the_write_is_kept_only_when_the_target_writes_nothing() {
    let quiet = command_with_id([1; 16], "wave the flag", vec![sound()]);
    let loud = command_with_id([2; 16], "shout", vec![write("Shout")]);
    // Runs the loud command: the target writes through the command it runs.
    let relay = command_with_id([3; 16], "relay", vec![exec_by_name("shout")]);
    // Two commands that run each other.
    let ping = command_with_id([4; 16], "ping", vec![exec_by_name("pong")]);
    let pong = command_with_id([5; 16], "pong", vec![exec_by_id([4; 16])]);
    let coder = command_with_id([6; 16], "coder", vec![plain_action(71)]);
    let cmds = vec![
        quiet,
        loud,
        relay,
        ping,
        pong,
        coder,
        command("by id", vec![write("A"), exec_by_id([1; 16])], 0, None),
        command(
            "by name",
            vec![write("B"), exec_by_name("wave the flag")],
            0,
            None,
        ),
        command(
            "to a writer",
            vec![write("C"), exec_by_id([2; 16])],
            0,
            None,
        ),
        command(
            "through a relay",
            vec![write("D"), exec_by_id([3; 16])],
            0,
            None,
        ),
        command(
            "into a cycle",
            vec![write("E"), exec_by_name("ping")],
            0,
            None,
        ),
        command("unknown id", vec![write("F"), exec_by_id([9; 16])], 0, None),
        command(
            "unknown name",
            vec![write("G"), exec_by_name("nobody")],
            0,
            None,
        ),
        command(
            "other case",
            vec![write("H"), exec_by_name("Wave The Flag")],
            0,
            None,
        ),
        command(
            "a variable name",
            vec![write("I"), exec_by_name("{TXT:x}")],
            0,
            None,
        ),
        command("to code", vec![write("J"), exec_by_id([6; 16])], 0, None),
    ];
    let b = built(cmds);
    assert!(mapped(&b, "by id"), "{:?}", b.report.left_out);
    assert!(mapped(&b, "by name"));
    assert!(mapped(&b, "into a cycle"));
    let writes = "action type 16 (execute a command) after the write: the command that it runs can write a request token";
    let unknown = "action type 16 (execute a command) after the write: the program cannot find the command that it runs";
    assert_eq!(not_understood(&b, "to a writer"), writes);
    assert_eq!(not_understood(&b, "through a relay"), writes);
    assert_eq!(
        not_understood(&b, "to code"),
        "action type 16 (execute a command) after the write: the command that it runs has code that is not reviewed"
    );
    assert_eq!(not_understood(&b, "unknown id"), unknown);
    assert_eq!(not_understood(&b, "unknown name"), unknown);
    assert_eq!(not_understood(&b, "other case"), unknown);
    assert_eq!(not_understood(&b, "a variable name"), unknown);
}

const SWITCHED_OFF: &str = "an action before the write is switched off in the profile, the \
                            program cannot know what VoiceAttack does";

#[test]
fn test_a_flagged_action_after_the_write_counts_as_present() {
    let b = built(vec![
        command(
            "raise the red flag",
            vec![write("RedFlag:Up"), flagged(write("RedFlag:Old"))],
            0,
            None,
        ),
        command(
            "raise the green flag",
            vec![write("GreenFlag:Up"), flagged(sound())],
            0,
            None,
        ),
        command(
            "raise the white flag",
            vec![write("WhiteFlag:Up"), flagged(exec_by_name("nobody"))],
            0,
            None,
        ),
    ]);
    assert_eq!(
        left(&b, "raise the red flag").unwrap().class,
        LeftOutClass::MultiWrite
    );
    assert!(mapped(&b, "raise the green flag"));
    // A flagged execute action after the write must pass its target rule.
    assert!(not_understood(&b, "raise the white flag").contains("cannot find"));
}

#[test]
fn test_flagged_action_at_or_before_the_write_leaves_the_command_out() {
    let b = built(vec![
        // A flagged single write.
        command(
            "raise the red flag",
            vec![sound(), flagged(write("RedFlag:Up"))],
            0,
            None,
        ),
        // A flagged inert action before the write.
        command(
            "raise the green flag",
            vec![flagged(sound()), write("GreenFlag:Up")],
            0,
            None,
        ),
        // A flagged set of a constant.
        command(
            "sail to the far island",
            vec![
                flagged(set_text("SystemName", "Far Island")),
                write("Sail:{TXT:SystemName}"),
            ],
            0,
            None,
        ),
        // A flagged set of a number variable.
        command(
            "raise the flag [1..9]",
            vec![
                set_text("LastCMD", "{CMD}"),
                flagged(set_text("Height", "{TXTNUM:LastCMD}")),
                write("Flag={TXT:Height}"),
            ],
            0,
            None,
        ),
        // A flagged action inside a condition block.
        command(
            "raise the black flag",
            vec![
                condition("x", "y", 6),
                flagged(say("ok")),
                end_condition(),
                write("BlackFlag:Up"),
            ],
            0,
            None,
        ),
    ]);
    for phrase in [
        "raise the red flag",
        "raise the green flag",
        "sail to the far island",
        "raise the flag [1..9]",
        "raise the black flag",
    ] {
        assert_eq!(not_understood(&b, phrase), SWITCHED_OFF, "{phrase}");
    }
}

#[test]
fn test_flagged_action_in_a_suffix_block_leaves_the_suffix_out() {
    let prefix = command(
        "fly the [blue; red]",
        vec![set_text("Size", "Big:")],
        1,
        Some("flags"),
    );
    let b = built(vec![
        prefix,
        command(
            "[now;] flag",
            vec![
                condition("{PREFIX}", "blue", 6),
                flagged(set_text("Colour", "Blue")),
                end_condition(),
                condition("Colour", "", 9),
                say("which flag"),
                else_branch(),
                write("{TXT:Size}{TXT:Colour}Flag"),
                end_condition(),
            ],
            2,
            Some("flags"),
        ),
    ]);
    assert_eq!(not_understood(&b, "[now;] flag"), SWITCHED_OFF);
}

#[test]
fn test_flagged_prefix_set_leaves_the_group_out() {
    let suffix = |phrase: &str, group: &str| {
        command(
            phrase,
            vec![
                condition("{PREFIX}", "blue", 6),
                set_text("Colour", "Blue"),
                end_condition(),
                condition("Colour", "", 9),
                say("which flag"),
                else_branch(),
                write("{TXT:Size}{TXT:Colour}Flag"),
                end_condition(),
            ],
            2,
            Some(group),
        )
    };
    let b = built(vec![
        command(
            "fly the [blue; red]",
            vec![flagged(set_text("Size", "Big:"))],
            1,
            Some("flags"),
        ),
        command(
            "hoist the [blue; red]",
            vec![set_text("Size", "Top:")],
            1,
            Some("flags"),
        ),
        suffix("[now;] flag", "flags"),
        command(
            "wave the [blue; red]",
            vec![set_text("Size", "Low:")],
            1,
            Some("pennants"),
        ),
        suffix("[now;] pennant", "pennants"),
    ]);
    assert_eq!(
        not_understood(&b, "fly the [blue; red]"),
        "an action of the prefix is switched off in the profile, the program cannot know what \
         VoiceAttack does"
    );
    let group = "a prefix of the composite group has an action that is switched off in the profile";
    assert_eq!(not_understood(&b, "hoist the [blue; red]"), group);
    assert_eq!(not_understood(&b, "[now;] flag"), group);
    // The other group stays.
    assert!(mapped(&b, "wave the [blue; red]"));
    assert!(mapped(&b, "[now;] pennant"));
}

#[test]
fn test_command_switched_off_or_without_voice_trigger_is_left_out() {
    let with_field = |phrase: &str, k: usize, v: Vec<u8>, actions: Vec<Vec<u8>>| {
        let mut f = command_fields(phrase, actions, 0, None);
        f[k] = v;
        record(f)
    };
    let mut target = command_fields("((wave the flag))", vec![sound()], 0, None);
    target[0] = vec![1; 16];
    target[15] = vec![1, 0];
    let b = built(vec![
        with_field("raise the red flag", 4, vec![0], vec![write("RedFlag:Up")]),
        with_field(
            "raise the green flag",
            15,
            vec![1, 0],
            vec![write("GreenFlag:Up")],
        ),
        // Such a command can still be the target of an execute action.
        record(target),
        flag_command(vec![write("BlueFlag:Up"), exec_by_id([1; 16])]),
    ]);
    let reason = "the command is switched off or has no voice trigger in the profile";
    assert_eq!(not_understood(&b, "raise the red flag"), reason);
    assert_eq!(not_understood(&b, "raise the green flag"), reason);
    assert!(mapped(&b, "raise the blue flag"), "{:?}", b.report.left_out);
}

fn blocks(built: &BuiltMap, phrase: &str) -> bool {
    built.blocking.iter().any(|p| p.phrase == phrase)
}

/// Fix round 10: when the program cannot read field 4 (switched on) or
/// field 15 (voice trigger), the command stays out of the map, but it keeps
/// its blocking phrase.
#[test]
fn test_an_unread_switch_or_voice_trigger_keeps_the_blocking_phrase() {
    let with_field = |phrase: &str, k: usize, v: Vec<u8>| {
        let mut f = command_fields(phrase, vec![write("Flag:Up")], 0, None);
        f[k] = v;
        record(f)
    };
    let b = built(vec![
        with_field("raise the red flag", 4, vec![1, 1]),
        with_field("raise the green flag", 15, vec![1, 1, 1]),
        with_field("raise the white flag", 4, vec![0]),
        flag_command(vec![write("BlueFlag:Up")]),
    ]);
    for (phrase, field) in [("raise the red flag", 4), ("raise the green flag", 15)] {
        assert_eq!(
            not_understood(&b, phrase),
            format!("the command has a setting in field {field} that the program does not know")
        );
        assert!(blocks(&b, phrase), "{phrase}");
    }
    // A field with the tested length keeps its meaning.
    assert!(!blocks(&b, "raise the white flag"));
    assert!(mapped(&b, "raise the blue flag"));
}

/// Fix round 10: a phrase longer than the limit gives a blocking phrase
/// from its first 4,096 characters, but only when the cut does not split
/// a group or a variable.
#[test]
fn test_a_long_phrase_blocks_with_its_first_characters_when_they_parse() {
    let max = extract::MAX_PHRASE_CHARS;
    let long = format!("[ring; toll] the bell{}", " now".repeat(max / 4));
    let split = format!("ring the bell [{}; soon]", "a".repeat(max));
    let open_var = format!("ring the bell {{TXT:{}}}", "b".repeat(max));
    let b = built(vec![
        command(&long, vec![write("Bell:1")], 0, None),
        command(&split, vec![write("Bell:2")], 0, None),
        command(&open_var, vec![write("Bell:3")], 0, None),
        flag_command(vec![write("BlueFlag:Up")]),
    ]);
    let reason = format!("the phrase is longer than {max} characters");
    for phrase in [&long, &split, &open_var] {
        assert_eq!(not_understood(&b, phrase), reason);
    }
    let first: String = long.chars().take(max).collect();
    assert!(blocks(&b, first.trim()), "{:?}", b.blocking.len());
    assert_eq!(b.blocking.len(), 1, "{:?}", b.blocking.len());
    assert!(mapped(&b, "raise the blue flag"));
}

#[test]
fn test_target_write_that_can_reach_the_request_file_leaves_the_caller_out() {
    let target = |path: &str| {
        let w = set_field(write("X"), 6, str_field(path));
        command_with_id([1; 16], "wave the flag", vec![w])
    };
    let caller = || flag_command(vec![write("BlueFlag:Up"), exec_by_id([1; 16])]);
    let writes = "action type 16 (execute a command) after the write: the command that it runs \
                  can write a request token";
    for path in [
        r"{TXT:PythonLibPath}\EDCoPilot.request.txt",
        r"{TXT:PythonLibPath}\{TXT:Name}.txt",
        r"C:\x\{TXT:Name}",
        r"{TXT:PythonLibPath}\flagfile.txt",
        "",
        // A literal name that is not in the list of known files.
        r"{TXT:PythonLibPath}\flag log.txt",
        // Windows names that can reach another file.
        r"{TXT:PythonLibPath}\.",
        r"{TXT:PythonLibPath}\..",
        r"{TXT:PythonLibPath}\EDCOPI~1.TXT",
        r"{TXT:PythonLibPath}\EDCoPilot.VApath.txt.",
        r"{TXT:PythonLibPath}\EDCoPilot.VApath.txt ",
        r"{TXT:PythonLibPath}\EDCoPilot.VApath.txt:stream",
        "{TXT:PythonLibPath}\\EDCoPilot.VApath.t\u{17f}t",
        // A known name in another folder, or with another separator.
        r"{TXT:PythonLibPath}\sub\EDCoPilot.VApath.txt",
        r"{TXT:PythonLibPath}\..\EDCoPilot.VApath.txt",
        r"{TXT:PythonLibPath}/EDCoPilot.VApath.txt",
        r"{TXT:PythonLibPath}EDCoPilot.VApath.txt",
        r"C:\x\EDCoPilot.VApath.txt",
    ] {
        let b = built(vec![
            target(path),
            caller(),
            // The profile sets the request file name to a literal.
            command(
                "init",
                vec![set_text("RequestFile", "FlagFile.txt")],
                0,
                None,
            ),
        ]);
        assert_eq!(
            not_understood(&b, "raise the blue flag"),
            writes,
            "{path:?}"
        );
    }
    // A known file name directly after the last placeholder is another
    // file. The case of the name does not matter.
    for path in [
        r"{TXT:PythonLibPath}\EDCoPilot.VApath.txt",
        r"{TXT:PythonLibPath}\launchedcopilot.BAT",
        r"{TXT:Other}\{TXT:PythonLibPath}\EDCoPilot.VAPversion.txt",
    ] {
        let b = built(vec![
            target(path),
            caller(),
            command(
                "init",
                vec![set_text("RequestFile", "FlagFile.txt")],
                0,
                None,
            ),
        ]);
        assert!(
            mapped(&b, "raise the blue flag"),
            "{path:?} {:?}",
            b.report.left_out
        );
    }
    // A known name that the profile gives to the request file.
    let b = built(vec![
        target(r"{TXT:PythonLibPath}\EDCoPilot.VApath.txt"),
        caller(),
        command(
            "init",
            vec![set_text("RequestFile", "edcopilot.vapath.txt")],
            0,
            None,
        ),
    ]);
    assert_eq!(not_understood(&b, "raise the blue flag"), writes);
}

#[test]
fn test_set_of_a_write_path_variable_leaves_the_command_out() {
    let reason = "the command changes the path of the request file";
    for set in [
        set_text("RequestFile", "flags.txt"),
        set_text("pythonlibpath", r"C:\flags"),
        set_text_from_file("RequestFile", "name.txt"),
    ] {
        let b = built(vec![flag_command(vec![set, write("BlueFlag:Up")])]);
        assert_eq!(not_understood(&b, "raise the blue flag"), reason);
    }
    let b = built(vec![flag_command(vec![
        condition("x", "y", 6),
        set_text("RequestFile", "flags.txt"),
        end_condition(),
        write("BlueFlag:Up"),
    ])]);
    assert_eq!(not_understood(&b, "raise the blue flag"), reason);
    // A prefix that sets one.
    let b = built(vec![command(
        "fly the [blue; red]",
        vec![set_text("RequestFile", "flags.txt")],
        1,
        Some("flags"),
    )]);
    assert_eq!(not_understood(&b, "fly the [blue; red]"), reason);
}

#[test]
fn test_slot_parse_only_at_depth_0_and_without_other_sets() {
    let cmd = |phrase: &str, before: Vec<Vec<u8>>, after: Vec<Vec<u8>>| {
        let mut acts = before;
        acts.extend(slot_parse(7));
        acts.extend(after);
        acts.push(write("Jump:{TXT:SystemName}"));
        command(phrase, acts, 0, None)
    };
    let b = built(vec![
        cmd("jump to {TXT:System7}", vec![], vec![]),
        // The shape inside a condition block.
        cmd(
            "hop to {TXT:System7}",
            vec![condition("x", "y", 6)],
            vec![end_condition()],
        ),
        // An extra set of a variable of the shape before it.
        cmd(
            "skip to {TXT:System7}",
            vec![set_text("DynamicVarLib", r"C:\other")],
            vec![],
        ),
        cmd(
            "leap to {TXT:System7}",
            vec![],
            vec![set_text("SysLength", "3")],
        ),
    ]);
    assert!(
        mapped(&b, "jump to {TXT:System7}"),
        "{:?}",
        b.report.left_out
    );
    not_understood(&b, "hop to {TXT:System7}");
    let reason = "the command sets a variable of the system-slot parse outside the parse";
    assert_eq!(not_understood(&b, "skip to {TXT:System7}"), reason);
    assert_eq!(not_understood(&b, "leap to {TXT:System7}"), reason);
    // The integer variables of the shape must have the tested names.
    let mut acts = slot_parse(7);
    acts[2] = set_field(acts[2].clone(), 14, str_field("Other"));
    acts.push(write("Jump:{TXT:SystemName}"));
    let b = built(vec![command("jump to {TXT:System7}", acts, 0, None)]);
    not_understood(&b, "jump to {TXT:System7}");
}

#[test]
fn test_number_phrase_with_a_point_or_a_minus_is_left_out() {
    let number = |phrase: &str| {
        command(
            phrase,
            vec![
                set_text("LastCMD", "{CMD}"),
                set_text("Height", "{TXTNUM:LastCMD}"),
                write("Flag={TXT:Height}"),
            ],
            0,
            None,
        )
    };
    let b = built(vec![
        number("raise the flag [1.50]"),
        number("lower the flag to minus-[1..9]"),
        number("raise the flag [1..9]"),
        // An ASCII digit outside a range, another numeral, a minus without
        // a twin, a minus with a twin.
        number("raise flag 2 to [1..9]"),
        number("raise flag \u{0663} to [1..9]"),
        number("[raise; up-raise] the flag [1..9]"),
        number("[upraise; up-raise] the flag [1..9]"),
        // A point outside a number command does not matter.
        command("raise the flag. now", vec![write("FlagNow")], 0, None),
    ]);
    let reason = crate::NUMBER_TEXT_REASON;
    assert_eq!(not_understood(&b, "raise the flag [1.50]"), reason);
    assert_eq!(not_understood(&b, "lower the flag to minus-[1..9]"), reason);
    assert!(mapped(&b, "raise flag 2 to [1..9]"));
    assert_eq!(not_understood(&b, "raise flag \u{0663} to [1..9]"), reason);
    assert_eq!(
        not_understood(&b, "[raise; up-raise] the flag [1..9]"),
        reason
    );
    assert!(mapped(&b, "[upraise; up-raise] the flag [1..9]"));
    assert!(mapped(&b, "raise the flag [1..9]"));
    assert!(mapped(&b, "raise the flag. now"));
}

#[test]
fn test_number_phrase_with_a_profile_variable_is_checked_with_its_grammar() {
    let number = |phrase: &str| {
        command(
            phrase,
            vec![
                set_text("LastCMD", "{CMD}"),
                set_text("Height", "{TXTNUM:LastCMD}"),
                write("Flag={TXT:Height}"),
            ],
            0,
            None,
        )
    };
    let b = built(vec![
        number("{TXT:FlagWords} [1..9]"),
        number("{TXT:MastWords} [1..9]"),
        command(
            "init flags",
            vec![set_text("FlagWords", "[raise; lift-up]")],
            0,
            None,
        ),
        command(
            "init masts",
            vec![set_text("MastWords", "[mast; spar]")],
            0,
            None,
        ),
    ]);
    assert_eq!(
        not_understood(&b, "{TXT:FlagWords} [1..9]"),
        crate::NUMBER_TEXT_REASON
    );
    assert!(mapped(&b, "{TXT:MastWords} [1..9]"));
    // The variable of the left-out record is not a profile variable now.
    assert_eq!(
        b.profile_vars,
        HashMap::from([("MastWords".to_string(), "[mast; spar]".to_string())])
    );
}

#[test]
fn test_number_suffix_is_left_out() {
    let b = built(vec![
        command(
            "fly the [blue; red]",
            vec![set_text("Size", "Big:")],
            1,
            Some("grp"),
        ),
        command(
            "[now;] flag [1..9]",
            vec![
                set_text("LastCMD", "{CMD}"),
                set_text("Height", "{TXTNUM:LastCMD}"),
                condition("{PREFIX}", "blue", 6),
                set_text("Colour", "Blue"),
                end_condition(),
                condition("Colour", "", 9),
                else_branch(),
                write("{TXT:Colour}{TXT:Size}Flag={TXT:Height}"),
                end_condition(),
            ],
            2,
            Some("grp"),
        ),
    ]);
    assert_eq!(
        not_understood(&b, "[now;] flag [1..9]"),
        "the number of a composite command can take digits of the prefix"
    );
}

/// A null string and an empty string are the same setting. A null list and
/// an empty list are the same setting.
#[test]
fn test_null_and_empty_settings_are_equal() {
    let b = built(vec![
        flag_command(vec![
            set_field(set_text("Colour", "blue"), 18, str_field("")),
            set_field(write("BlueFlag:Up"), 30, null_str()),
        ]),
        command(
            "raise the red flag",
            vec![set_field(write("RedFlag:Up"), 9, int_field(5))],
            0,
            None,
        ),
    ]);
    assert!(mapped(&b, "raise the blue flag"), "{:?}", b.report.left_out);
    assert_eq!(
        not_understood(&b, "raise the red flag"),
        "an action has a setting that the program does not know"
    );
}

/// Action field 9 is a 4-byte integer (-1 in the tested profile), not a
/// string. The value 0 is another setting, not an empty string.
#[test]
fn test_field_9_zero_is_another_setting() {
    let b = built(vec![flag_command(vec![
        set_field(set_text("Colour", "blue"), 9, int_field(0)),
        write("BlueFlag:Up"),
    ])]);
    assert_eq!(
        not_understood(&b, "raise the blue flag"),
        "an action has a setting that the program does not know"
    );
}

/// The report counts the actions with an unknown setting by type and field,
/// also for a type that the builder does not model (its shared fields).
#[test]
fn test_report_counts_the_unknown_settings() {
    let b = built(vec![command(
        "raise the red flag",
        vec![
            set_field(set_text("Colour", "red"), 4, int_field(1)),
            set_field(set_text("Colour", "red"), 4, int_field(2)),
            set_field(sound(), 29, int_field(1)),
            set_field(sound(), 4, int_field(1)),
            write("RedFlag:Up"),
        ],
        0,
        None,
    )]);
    assert_eq!(
        b.report.unknown_settings,
        std::collections::BTreeMap::from([((21, 4), 2), ((14, 29), 1)])
    );
}

/// Many actions with one new value in a shared field: the format changed,
/// and the build stops. A few such actions leave only their commands out.
#[test]
fn test_a_shared_field_change_of_many_actions_stops_the_build() {
    let stamp = |n: usize| -> Vec<Vec<u8>> {
        (0..n)
            .map(|k| {
                command(
                    &format!("hoist pennant number {k}"),
                    vec![set_field(write(&format!("Pennant:{k}")), 26, vec![1; 12])],
                    0,
                    None,
                )
            })
            .collect()
    };
    // 60 of 260 actions: more than 5 percent and at least 50.
    let e = build_map(&vap(stamp(60))).unwrap_err();
    assert_eq!(
        e,
        BuildError::FormatChanged {
            action_type: None,
            kind: RecordKind::Action,
            field: 26,
            records: 60,
            total: 260
        }
    );
    assert_eq!(
        e.to_string(),
        "the format of the profile is not understood: 60 of 260 actions have another value \
         in action field 26, the profile has a newer format than the tested format, tell the \
         developer"
    );
    // 12 of 212 actions: more than 5 percent, but fewer than 50.
    let b = built(stamp(12));
    assert_eq!(b.report.left_out_count(LeftOutClass::NotUnderstood), 12);
    // 50 actions, but not more than 5 percent of the actions.
    let mut many = stamp(50);
    many.extend(filler(1000));
    let b = build_map(&deflate(&profile(many))).unwrap();
    assert_eq!(b.report.left_out_count(LeftOutClass::NotUnderstood), 50);
}

/// Many actions with other values in one shared field, each value on a
/// few actions only: the format changed too.
#[test]
fn test_a_shared_field_change_with_many_values_stops_the_build() {
    let stamp = |n: usize| -> Vec<Vec<u8>> {
        (0..n)
            .map(|k| {
                let mut value = vec![0; 12];
                value[..8].copy_from_slice(&(k as u64 + 1).to_le_bytes());
                command(
                    &format!("hoist pennant number {k}"),
                    vec![set_field(write(&format!("Pennant:{k}")), 26, value)],
                    0,
                    None,
                )
            })
            .collect()
    };
    assert_eq!(
        build_map(&vap(stamp(60))).unwrap_err(),
        BuildError::FormatChanged {
            action_type: None,
            kind: RecordKind::Action,
            field: 26,
            records: 60,
            total: 260
        }
    );
    // 49 actions: fewer than 50. Only their commands leave the map.
    let b = built(stamp(49));
    assert_eq!(b.report.left_out_count(LeftOutClass::NotUnderstood), 49);
}

#[test]
fn test_unknown_setting_of_a_modelled_action_leaves_the_command_out() {
    let reason = "an action has a setting that the program does not know";
    let b = built(vec![
        flag_command(vec![set_field(write("BlueFlag:Up"), 11, int_field(1))]),
        command(
            "raise the red flag",
            vec![
                set_field(set_text("Colour", "red"), 4, int_field(1)),
                write("RedFlag:Up"),
            ],
            0,
            None,
        ),
        // A set-text after the write is modelled too.
        command(
            "raise the green flag",
            vec![
                write("GreenFlag:Up"),
                set_field(set_text("Colour", "x"), 26, vec![0; 12]),
            ],
            0,
            None,
        ),
        // A type that the builder does not model: no check.
        command(
            "raise the white flag",
            vec![set_field(sound(), 4, int_field(1)), write("WhiteFlag:Up")],
            0,
            None,
        ),
    ]);
    assert_eq!(not_understood(&b, "raise the blue flag"), reason);
    assert_eq!(not_understood(&b, "raise the red flag"), reason);
    assert_eq!(not_understood(&b, "raise the green flag"), reason);
    assert!(mapped(&b, "raise the white flag"));
    // A {PREFIX} block without its tested field 12.
    let mut block = fields_of(&condition("{PREFIX}", "blue", 6));
    block[12] = int_field(0);
    let b = built(vec![
        command(
            "fly the [blue; red]",
            vec![set_text("Size", "Big:")],
            1,
            Some("flags"),
        ),
        command(
            "[now;] flag",
            vec![
                record(block),
                set_text("Colour", "Blue"),
                end_condition(),
                condition("Colour", "", 9),
                say("which flag"),
                else_branch(),
                write("{TXT:Size}{TXT:Colour}Flag"),
                end_condition(),
            ],
            2,
            Some("flags"),
        ),
    ]);
    assert!(not_understood(&b, "[now;] flag").starts_with("the suffix"));
}

#[test]
fn test_execute_mode_chooses_id_or_name() {
    let unknown = "action type 16 (execute a command) after the write: the program cannot find \
                   the command that it runs";
    let b = built(vec![
        command_with_id([1; 16], "wave the flag", vec![sound()]),
        command_with_id([2; 16], "dip the flag", vec![sound()]),
        // By id, with a name of the same command.
        command(
            "one",
            vec![
                write("One"),
                exec_with([1; 16], Some("wave the flag"), false),
            ],
            0,
            None,
        ),
        // By id, with a name of another command.
        command(
            "two",
            vec![
                write("Two"),
                exec_with([1; 16], Some("dip the flag"), false),
            ],
            0,
            None,
        ),
        // By name, but an id is present.
        command(
            "three",
            vec![
                write("Three"),
                exec_with([1; 16], Some("wave the flag"), true),
            ],
            0,
            None,
        ),
        // By id, but the id is zero.
        command(
            "four",
            vec![
                write("Four"),
                exec_with([0; 16], Some("wave the flag"), false),
            ],
            0,
            None,
        ),
        // A mode that is not known.
        command(
            "five",
            vec![
                write("Five"),
                set_field(exec_by_id([1; 16]), 3, 2f64.to_le_bytes().to_vec()),
            ],
            0,
            None,
        ),
    ]);
    assert!(mapped(&b, "one"), "{:?}", b.report.left_out);
    for phrase in ["two", "three", "four", "five"] {
        assert_eq!(not_understood(&b, phrase), unknown, "{phrase}");
    }
}

#[test]
fn test_suffix_not_set_block_must_check_a_rule_variable_of_the_token() {
    let suffix = |checked: &str, token: &str| {
        command(
            "[now;] flag",
            vec![
                condition("{PREFIX}", "blue", 6),
                set_text("Colour", "Blue"),
                end_condition(),
                condition(checked, "", 9),
                say("which flag"),
                else_branch(),
                write(token),
                end_condition(),
            ],
            2,
            Some("flags"),
        )
    };
    let prefix = || {
        command(
            "fly the [blue; red]",
            vec![set_text("Size", "Big:")],
            1,
            Some("flags"),
        )
    };
    let b = built(vec![
        prefix(),
        suffix("Colour", "{TXT:Size}{TXT:Colour}Flag"),
    ]);
    assert!(mapped(&b, "[now;] flag"), "{:?}", b.report.left_out);
    for (checked, token) in [
        ("Wind", "{TXT:Size}{TXT:Colour}Flag"),
        ("Size", "{TXT:Size}{TXT:Colour}Flag"),
        ("Colour", "{TXT:Size}Flag"),
    ] {
        let b = built(vec![prefix(), suffix(checked, token)]);
        assert!(
            not_understood(&b, "[now;] flag").starts_with("the suffix"),
            "{checked}"
        );
    }
}

/// Fix round 10: a variable that a `{PREFIX}` block sets and that a plain
/// set outside the blocks also sets is not understood, in each order. A
/// suffix whose rules alone set the variable keeps its build.
#[test]
fn test_a_rule_variable_with_a_set_outside_the_prefix_blocks_leaves_the_suffix_out() {
    let prefix = || {
        command(
            "fly the [blue; red]",
            vec![set_text("Size", "Big:")],
            1,
            Some("flags"),
        )
    };
    let suffix = |before: bool, after: bool| {
        let mut actions = Vec::new();
        if before {
            actions.push(set_text("SystemName", "Cove"));
        }
        actions.extend([
            condition("{PREFIX}", "blue", 6),
            set_text("SystemName", "Bay"),
            end_condition(),
        ]);
        if after {
            actions.push(set_text("SystemName", "Cove"));
        }
        actions.extend([
            condition("SystemName", "", 9),
            else_branch(),
            write("Go:{TXT:SystemName}"),
            end_condition(),
        ]);
        command("[now;] flag", actions, 2, Some("flags"))
    };
    let b = built(vec![prefix(), suffix(false, false)]);
    assert!(mapped(&b, "[now;] flag"), "{:?}", b.report.left_out);
    let rec = record_of_phrase(&b, "[now;] flag");
    assert!(rec.constants.is_empty(), "{:?}", rec.constants);
    assert_eq!(rec.prefix_rules.len(), 1);
    for (before, after) in [(true, false), (false, true)] {
        let b = built(vec![prefix(), suffix(before, after)]);
        assert_eq!(
            not_understood(&b, "[now;] flag"),
            "the suffix sets a variable of its prefix rules outside the {PREFIX} blocks",
            "before {before}, after {after}"
        );
    }
}

#[test]
fn test_names_that_differ_only_in_case_leave_the_part_out() {
    let b = built(vec![
        command(
            "fly the [blue; red]",
            vec![set_text("Size", "Big:"), set_text("size", "Top:")],
            1,
            Some("flags"),
        ),
        command(
            "[now;] flag",
            vec![
                condition("{PREFIX}", "blue", 6),
                set_text("Colour", "Blue"),
                end_condition(),
                condition("{PREFIX}", "red", 6),
                set_text("colour", "Red"),
                end_condition(),
                condition("Colour", "", 9),
                say("which flag"),
                else_branch(),
                write("{TXT:Colour}Flag"),
                end_condition(),
            ],
            2,
            Some("flags"),
        ),
    ]);
    assert_eq!(
        not_understood(&b, "fly the [blue; red]"),
        "the prefix sets variables whose names differ only in case"
    );
    assert_eq!(
        not_understood(&b, "[now;] flag"),
        "the suffix sets variables whose names differ only in case"
    );
}

#[test]
fn test_profile_variable_with_a_non_literal_set_or_code_is_not_known() {
    let user = |var: &str| {
        command(
            &format!("{{TXT:{var}}} the flag"),
            vec![write("Flag:{CMD}")],
            0,
            None,
        )
    };
    let b = built(vec![
        user("Places"),
        command(
            "init places",
            vec![set_text("Places", "[raise; lower]")],
            0,
            None,
        ),
        user("Moods"),
        command(
            "init moods",
            vec![set_text("Moods", "[calm; wild]")],
            0,
            None,
        ),
        command(
            "read moods",
            vec![set_text_from_file("Moods", "moods.txt")],
            0,
            None,
        ),
        user("Winds"),
        command(
            "init winds",
            vec![set_text("Winds", "[east; west]")],
            0,
            None,
        ),
        command(
            "code winds",
            vec![inline_code("VA.SetText(\"Winds\", x);")],
            0,
            None,
        ),
        // The only set has a replace field: not a literal grammar.
        user("Sails"),
        command(
            "init sails",
            vec![set_text_replace("Sails", "[full; half]", "x")],
            0,
            None,
        ),
        user("Tides"),
        command(
            "init tides",
            vec![set_text("Tides", "[high; low]")],
            0,
            None,
        ),
        command(
            "plugin tides",
            vec![set_field(plain_action(22), 8, str_field("{TXT:tides}"))],
            0,
            None,
        ),
    ]);
    assert_eq!(
        b.profile_vars,
        HashMap::from([("Places".to_string(), "[raise; lower]".to_string())])
    );
}

#[test]
fn test_format_note_names_the_smallest_and_the_largest_count() {
    let mut counts = ObservedCounts {
        profile: 90,
        ..ObservedCounts::default()
    };
    counts.command = [82, 83, 84].into();
    counts.action = [32, 33].into();
    let text = FormatNote::from_counts(&counts).to_string();
    assert!(text.contains("command 82 to 84"), "{text}");
    assert!(text.contains("action 32 to 33"), "{text}");
    assert!(text.contains("profile 90"), "{text}");
}

#[test]
fn test_unknown_operator_leaves_one_command_out_and_keeps_the_others() {
    let b = built(vec![
        flag_command(vec![
            condition("x", "y", 7),
            say("odd"),
            end_condition(),
            write("BlueFlag:Up"),
        ]),
        command(
            "raise the red flag",
            vec![
                condition("x", "y", 9),
                say("ok"),
                end_condition(),
                write("RedFlag:Up"),
            ],
            0,
            None,
        ),
    ]);
    assert_eq!(
        not_understood(&b, "raise the blue flag"),
        "a condition uses the operator 7, which the program does not know"
    );
    assert!(mapped(&b, "raise the red flag"));
}

#[test]
fn test_unknown_command_type_and_category_phrase_leave_the_command_out() {
    let b = built(vec![
        command("raise the blue flag", vec![write("BlueFlag:Up")], 3, None),
        command("cat", vec![write("Cat")], 0, None),
    ]);
    assert_eq!(
        not_understood(&b, "raise the blue flag"),
        "the command type 3 is not known"
    );
    assert_eq!(
        not_understood(&b, "cat"),
        "the phrase is also the name of a category"
    );
}

#[test]
fn test_slot_phrase_with_the_file_of_another_slot_is_left_out() {
    let cmd = |phrase: &str, file: u32| {
        let mut acts = slot_parse(file);
        acts.push(write("Jump:{TXT:SystemName}"));
        command(phrase, acts, 0, None)
    };
    let b = built(vec![
        cmd("jump to {TXT:System7}", 7),
        cmd("hop to {TXT:System8}", 7),
    ]);
    assert!(
        mapped(&b, "jump to {TXT:System7}"),
        "{:?}",
        b.report.left_out
    );
    assert_eq!(
        not_understood(&b, "hop to {TXT:System8}"),
        "the command reads the slot file 7, but its phrase slot has another number"
    );
    // The token takes the slot from the fallback, but the command does not
    // read the slot file.
    let b = built(vec![command(
        "go to {TXT:System7}",
        vec![write("Go:{TXT:SystemName}")],
        0,
        None,
    )]);
    assert_eq!(
        not_understood(&b, "go to {TXT:System7}"),
        "the token takes the spoken slot, but the command does not read the slot file"
    );
}

#[test]
fn test_prefix_with_a_condition_set_is_left_out() {
    let b = built(vec![
        command(
            "fly the [blue; red]",
            vec![
                condition("x", "y", 6),
                set_text("Size", "Big:"),
                end_condition(),
            ],
            1,
            Some("flags"),
        ),
        command(
            "fly the [green; white]",
            vec![set_text("Size", "{TXT:x}")],
            1,
            Some("flags"),
        ),
    ]);
    let reason = "the prefix sets a variable in a way that the program does not model";
    assert_eq!(not_understood(&b, "fly the [blue; red]"), reason);
    assert_eq!(not_understood(&b, "fly the [green; white]"), reason);
}

#[test]
fn test_too_few_records_is_an_error() {
    let mut cmds = filler(199);
    cmds.push(command("stop", vec![sound()], 0, None));
    let err = build_map(&deflate(&profile(cmds)));
    assert_eq!(
        err.unwrap_err(),
        BuildError::Totals(Totals::TooFewRecords {
            found: 199,
            minimum: 200,
            largest_reason: Some(("the command writes no request token".into(), 1)),
        })
    );
    // 200 records, but none with a fixed token.
    let cmds = (0..200)
        .map(|k| command(&format!("go {k}"), vec![write("Go:{CMD}")], 0, None))
        .collect();
    let err = build_map(&deflate(&profile(cmds)));
    assert_eq!(err.unwrap_err(), BuildError::Totals(Totals::NoFixedRecord));
}

#[test]
fn test_report_difference_names_added_removed_and_moved() {
    let old = built(vec![
        flag_command(vec![write("BlueFlag:Up")]),
        command("raise the red flag", vec![sound()], 0, None),
        command("lower the flag", vec![write("Lower")], 0, None),
        command("wave the flag", vec![write("Wave")], 0, None),
    ]);
    let new = built(vec![
        flag_command(vec![plain_action(71), write("BlueFlag:Up")]),
        command("raise the red flag", vec![write("RedFlag:Up")], 0, None),
        command("fold the flag", vec![write("Fold")], 0, None),
        command("wave the flag", vec![write("Wave")], 0, None),
    ]);
    let d = report_difference(&old.report, &new.report);
    assert_eq!(d.added, ["fold the flag"]);
    assert_eq!(d.removed, ["lower the flag"]);
    assert_eq!(d.moved_to_left_out, ["raise the blue flag"]);
    assert_eq!(d.moved_to_map, ["raise the red flag"]);
    assert!(!d.is_empty());
    assert!(report_difference(&new.report, &new.report).is_empty());
}

#[test]
fn test_text_file_zip_file_cut_file_and_empty_file_are_typed_errors() {
    let good = vap(vec![flag_command(vec![write("BlueFlag:Up")])]);
    let zip = [
        b"PK\x03\x04".as_slice(),
        &[0x14, 0, 0, 0, 8, 0],
        &good[..40],
    ]
    .concat();
    let cases: [(&str, Vec<u8>); 4] = [
        (
            "text",
            b"this is a text file\r\nwith two lines\r\n".to_vec(),
        ),
        ("zip", zip),
        ("cut", good[..good.len() / 2].to_vec()),
        ("empty", Vec::new()),
    ];
    for (name, bytes) in cases {
        let err = build_map(&bytes).unwrap_err();
        assert!(
            matches!(
                err,
                BuildError::Inflate { .. }
                    | BuildError::Truncated
                    | BuildError::TrailingBytes { .. }
                    | BuildError::RootSize { .. }
            ),
            "{name}: {err:?}"
        );
        let text = err.to_string();
        assert!(!text.is_empty() && !text.contains('{'), "{name}: {text}");
    }
    assert_eq!(build_map(&[]).unwrap_err(), BuildError::Truncated);
    // A valid stream with content that is not a profile.
    let err = build_map(&deflate(b"hello, this is not a profile")).unwrap_err();
    assert!(matches!(err, BuildError::RootSize { .. }), "{err:?}");
}

/// A small deterministic generator (xorshift), so that the test needs no
/// new dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

#[test]
fn test_mutated_profiles_never_panic_and_never_allocate_a_read_length() {
    let raw = profile(vec![
        flag_command(vec![sound(), write("BlueFlag:Up")]),
        command(
            "fly the [blue; red]",
            vec![set_text("Size", "Big:")],
            1,
            Some("flags"),
        ),
        command(
            "[now;] flag",
            vec![
                condition("{PREFIX}", "blue", 6),
                set_text("Colour", "Blue"),
                end_condition(),
                condition("Colour", "", 9),
                say("which flag"),
                else_branch(),
                write("{TXT:Size}{TXT:Colour}Flag"),
                end_condition(),
            ],
            2,
            Some("flags"),
        ),
        command_with_id([1; 16], "wave", vec![write("W"), exec_by_name("wave")]),
        command(
            "jump to {TXT:System7}",
            {
                let mut a = slot_parse(7);
                a.push(write("Jump:{TXT:SystemName}"));
                a
            },
            0,
            None,
        ),
    ]);
    let packed = deflate(&raw);
    let mut rng = Rng(0x5eed_1234_abcd_0001);
    let mut outcomes = [0usize; 2];
    for round in 0..3000 {
        // Mutate the inflated bytes (to reach the walk and the rules) or the
        // compressed bytes (to reach the inflate step).
        let inflated_side = round % 4 != 0;
        let mut bytes = if inflated_side {
            raw.clone()
        } else {
            packed.clone()
        };
        for _ in 0..1 + rng.below(4) {
            match rng.below(4) {
                0 => {
                    let at = rng.below(bytes.len());
                    bytes[at] ^= 1 << rng.below(8);
                }
                1 => {
                    let at = rng.below(bytes.len());
                    bytes[at] = rng.next() as u8;
                }
                2 => {
                    // A large length or count at a 4-byte position.
                    let at = rng.below(bytes.len().saturating_sub(4).max(1));
                    let v = [i32::MAX, i32::MIN, -1, 0x7fff_fff0, 1 << 24][rng.below(5)];
                    if at + 4 <= bytes.len() {
                        bytes[at..at + 4].copy_from_slice(&v.to_le_bytes());
                    }
                }
                _ => {
                    let keep = rng.below(bytes.len() + 1);
                    bytes.truncate(keep);
                }
            }
            if bytes.is_empty() {
                break;
            }
        }
        let input = if inflated_side {
            deflate(&bytes)
        } else {
            bytes
        };
        outcomes[build_map(&input).is_ok() as usize] += 1;
    }
    // The profile has fewer than 200 records, so no mutation builds.
    assert_eq!(outcomes, [3000, 0]);
    // A huge value at each position that holds a length, a count, a size
    // or an offset (a small positive int32).
    let mut swept = 0;
    for at in 0..raw.len() - 4 {
        let v = i32::from_le_bytes(raw[at..at + 4].try_into().unwrap());
        if !(1..=1 << 16).contains(&v) {
            continue;
        }
        for huge in [i32::MAX, 0x0fff_ffff] {
            let mut bytes = raw.clone();
            bytes[at..at + 4].copy_from_slice(&huge.to_le_bytes());
            assert!(build_map(&deflate(&bytes)).is_err());
            swept += 1;
        }
    }
    assert!(swept > 500, "{swept}");
    // A root record with a huge header count and all offsets equal.
    for n in [i32::MAX, 1 << 20, 1024, 1000] {
        let table_end = 8 + 4 * (n.min(1024) as usize + 1);
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&(table_end as i32).to_le_bytes());
        bytes.extend_from_slice(&n.to_le_bytes());
        while bytes.len() < table_end {
            bytes.extend_from_slice(&(table_end as i32).to_le_bytes());
        }
        assert!(build_map(&deflate(&bytes)).is_err(), "{n}");
    }
    // No allocation follows a length or a count from the file. The largest
    // request stays far below the 2 GiB that such a value could ask for.
    assert!(peak_request() < 80 * 1024 * 1024, "{}", peak_request());
}

/// The command fields after field 20 with one value in the tested
/// profile: another value leaves the command out. A null string and an
/// empty string are equal in the fields that hold a null string.
#[test]
fn test_unknown_command_setting_after_field_20_leaves_the_command_out() {
    let with = |phrase: &str, field: usize, value: Vec<u8>| {
        let mut f = command_fields(phrase, vec![write(&format!("Flag:{phrase}"))], 0, None);
        f[field] = value;
        record(f)
    };
    let cases = [
        ("raise flag a", 23, int_field(1)),
        ("raise flag b", 24, vec![1]),
        ("raise flag c", 36, vec![0]),
        ("raise flag d", 48, vec![1, 0]),
        ("raise flag e", 58, int_field(0)),
        ("raise flag f", 66, [vec![0; 16], vec![1]].concat()),
        ("raise flag g", 75, int_field(-1)),
        ("raise flag h", 81, vec![1]),
        ("raise flag i", 5, str_field("x")),
        ("raise flag j", 82, int_field(1)),
    ];
    let b = built(
        cases
            .iter()
            .map(|(p, f, v)| with(p, *f, v.clone()))
            .chain([
                with("raise flag empty", 5, str_field("")),
                with("raise flag empty two", 80, str_field("")),
            ])
            .collect(),
    );
    for (phrase, field, _) in &cases {
        assert_eq!(
            not_understood(&b, phrase),
            format!("the command has a setting in field {field} that the program does not know")
        );
    }
    assert!(mapped(&b, "raise flag empty"));
    assert!(mapped(&b, "raise flag empty two"));
}

/// Many commands with another value in one command setting: the format
/// changed, and the build stops.
#[test]
fn test_a_command_setting_change_of_many_commands_stops_the_build() {
    let changed = |n: usize| -> Vec<Vec<u8>> {
        (0..n)
            .map(|k| {
                let mut f = command_fields(
                    &format!("hoist pennant number {k}"),
                    vec![write(&format!("Pennant:{k}"))],
                    0,
                    None,
                );
                f[57] = int_field(k as i32 + 1);
                record(f)
            })
            .collect()
    };
    // 60 of 260 commands: more than 5 percent and at least 50.
    let e = build_map(&vap(changed(60))).unwrap_err();
    assert_eq!(
        e,
        BuildError::FormatChanged {
            action_type: None,
            kind: RecordKind::Command,
            field: 57,
            records: 60,
            total: 260
        }
    );
    assert_eq!(
        e.to_string(),
        "the format of the profile is not understood: 60 of 260 commands have another value \
         in command field 57, the profile has a newer format than the tested format, tell the \
         developer"
    );
    // 49 commands: only these commands leave the map.
    let b = built(changed(49));
    assert_eq!(b.report.left_out_count(LeftOutClass::NotUnderstood), 49);
}

/// A command setting (fields 8, 10 to 13, 16 to 18) with another value
/// than in the tested profile leaves the command out. The other commands
/// load.
#[test]
fn test_unknown_command_setting_leaves_the_command_out() {
    let with = |phrase: &str, field: usize, value: Vec<u8>| {
        let mut f = command_fields(phrase, vec![write(&format!("Flag:{phrase}"))], 0, None);
        f[field] = value;
        record(f)
    };
    let cases = [
        ("raise flag eight", 8, vec![1]),
        ("raise flag ten", 10, int_field(1)),
        ("raise flag eleven", 11, int_field(3)),
        ("raise flag twelve", 12, int_field(1)),
        ("raise flag thirteen", 13, int_field(1)),
        ("raise flag sixteen", 16, vec![1]),
        ("raise flag seventeen", 17, int_field(0)),
        ("raise flag eighteen", 18, int_field(1)),
    ];
    let b = built(
        cases
            .iter()
            .map(|(p, f, v)| with(p, *f, v.clone()))
            .chain([flag_command(vec![write("BlueFlag:Up")])])
            .collect(),
    );
    for (phrase, field, _) in &cases {
        assert_eq!(
            not_understood(&b, phrase),
            format!("the command has a setting in field {field} that the program does not know")
        );
    }
    assert!(mapped(&b, "raise the blue flag"));
    // A prefix with such a setting.
    let mut prefix = command_fields(
        "fly the [blue; red]",
        vec![set_text("Size", "Big:")],
        1,
        Some("g"),
    );
    prefix[10] = int_field(1);
    let b = built(vec![record(prefix)]);
    assert_eq!(
        not_understood(&b, "fly the [blue; red]"),
        "the command has a setting in field 10 that the program does not know"
    );
    // The target of a type 16 action.
    let mut target = command_fields("wave the flag", vec![sound()], 0, None);
    target[0] = vec![1; 16];
    target[12] = int_field(1);
    let b = built(vec![
        record(target),
        flag_command(vec![write("BlueFlag:Up"), exec_by_id([1; 16])]),
    ]);
    assert_eq!(
        not_understood(&b, "raise the blue flag"),
        "action type 16 (execute a command) after the write: the command that it runs has a \
         setting that the program does not know"
    );
}

/// A set action whose variable name has a token can set any variable,
/// also a part of the token. At or before the write, it leaves the
/// command out. After the write, the token is already written.
#[test]
fn test_set_with_a_token_in_the_variable_name_leaves_the_command_out() {
    let reason = "a set action at or before the write has a variable name with a token, the \
                  program cannot know which variable it sets";
    let named = |kind: i32, field: usize| {
        let mut f = action_fields(kind);
        f[field] = str_field("Flag{TXT:Which}");
        record(f)
    };
    let sets = [
        set_text("Flag{TXT:Which}", "x"),
        named(36, 5),
        named(37, 14),
        named(18, 14),
        named(38, 14),
    ];
    for set in sets.clone() {
        let b = built(vec![flag_command(vec![set, write("BlueFlag:Up")])]);
        assert_eq!(not_understood(&b, "raise the blue flag"), reason);
    }
    // In a condition block.
    let b = built(vec![flag_command(vec![
        condition("x", "y", 6),
        set_text("Flag{TXT:Which}", "x"),
        end_condition(),
        write("BlueFlag:Up"),
    ])]);
    assert_eq!(not_understood(&b, "raise the blue flag"), reason);
    // After the write.
    for set in sets {
        let b = built(vec![flag_command(vec![write("BlueFlag:Up"), set])]);
        assert!(mapped(&b, "raise the blue flag"), "{:?}", b.report.left_out);
    }
    // In a prefix.
    let b = built(vec![command(
        "fly the [blue; red]",
        vec![set_text("Size", "Big:"), named(37, 14)],
        1,
        Some("g"),
    )]);
    assert_eq!(not_understood(&b, "fly the [blue; red]"), reason);
    // In the shape of a suffix.
    let b = built(vec![
        command(
            "fly the [blue; red]",
            vec![set_text("Size", "Big:")],
            1,
            Some("grp"),
        ),
        command(
            "[now;] flag",
            vec![
                condition("{PREFIX}", "blue", 6),
                set_text("Colour", "Blue"),
                end_condition(),
                condition("Colour", "", 9),
                named(36, 5),
                else_branch(),
                write("{TXT:Colour}{TXT:Size}Flag"),
                end_condition(),
            ],
            2,
            Some("grp"),
        ),
    ]);
    assert!(left(&b, "[now;] flag").is_some());
    assert!(!mapped(&b, "[now;] flag"));
}

/// A variable that a type 77 or type 74 action writes is not a profile
/// variable.
#[test]
fn test_a_variable_that_type_77_or_74_writes_is_not_a_profile_variable() {
    let writer = |kind: i32, name: &str| {
        let mut f = action_fields(kind);
        f[6] = str_field(name);
        record(f)
    };
    for kind in [77, 74] {
        let b = built(vec![
            command("{TXT:FlagWords} now", vec![write("Flag:{CMD}")], 0, None),
            command(
                "init flags",
                vec![set_text("FlagWords", "[raise; lift]")],
                0,
                None,
            ),
            command("other", vec![writer(kind, "flagwords")], 0, None),
        ]);
        assert!(b.profile_vars.is_empty(), "type {kind}");
    }
}

/// An execute action by name runs the command whose phrase VoiceAttack
/// matches. Another command whose phrase gives the same name by its
/// alternatives or its variables makes the target not known.
#[test]
fn test_execute_by_name_refuses_a_name_that_another_phrase_gives() {
    let unknown = "action type 16 (execute a command) after the write: the program cannot find \
                   the command that it runs";
    let caller = || flag_command(vec![write("BlueFlag:Up"), exec_by_name("wave the flag")]);
    let target = || command("wave the flag", vec![sound()], 0, None);
    for other in [
        "[wave; dip] the flag",
        "[now;] wave the flag",
        "Wave [the;] FLAG",
        "dip the flag; wave the flag ",
        "{TXT:Mood} the flag",
        "wave the [flag; [pennant; banner]]",
        "wave the flag [1..9;]",
    ] {
        let b = built(vec![
            target(),
            caller(),
            command(other, vec![sound()], 0, None),
        ]);
        assert_eq!(
            not_understood(&b, "raise the blue flag"),
            unknown,
            "{other:?}"
        );
    }
    for other in [
        "[raise; lower] the flag",
        "wave the [pennant; banner]",
        "wave the flag [1..9]",
        "[wave; dip] the flag now",
    ] {
        let b = built(vec![
            target(),
            caller(),
            command(other, vec![sound()], 0, None),
        ]);
        assert!(
            mapped(&b, "raise the blue flag"),
            "{other:?} {:?}",
            b.report.left_out
        );
    }
    // A phrase that the builder cannot read: the target is not known.
    let b = built(vec![
        target(),
        caller(),
        command("wave [the flag", vec![sound()], 0, None),
    ]);
    assert_eq!(not_understood(&b, "raise the blue flag"), unknown);
}

/// Two phrases that differ only in spacing, case or punctuation are one
/// name for an execute action by name: the target is not known. A target
/// whose phrase does not have single spaces is not known either.
#[test]
fn test_execute_by_name_refuses_names_that_differ_only_in_form() {
    let unknown = "action type 16 (execute a command) after the write: the program cannot find \
                   the command that it runs";
    let caller = || flag_command(vec![write("BlueFlag:Up"), exec_by_name("wave the flag")]);
    for other in [
        "wave  the flag",
        "Wave The Flag!",
        "wave, the flag",
        " wave the-flag",
    ] {
        let b = built(vec![
            command("wave the flag", vec![sound()], 0, None),
            caller(),
            command(other, vec![sound()], 0, None),
        ]);
        assert_eq!(
            not_understood(&b, "raise the blue flag"),
            unknown,
            "{other:?}"
        );
    }
    let b = built(vec![
        command("wave  the flag", vec![sound()], 0, None),
        flag_command(vec![write("BlueFlag:Up"), exec_by_name("wave  the flag")]),
    ]);
    assert_eq!(not_understood(&b, "raise the blue flag"), unknown);
}

/// The profile of the tests of the name patterns: the profile variable
/// `var` with one literal set, a command that uses it, and `more`.
fn lamp_commands(var: &str, more: Vec<Vec<u8>>) -> Vec<Vec<u8>> {
    let mut cmds = vec![
        command(
            &format!("{{TXT:{var}}} the lighthouse"),
            vec![write("Lamp:{CMD}")],
            0,
            None,
        ),
        command(
            "init lamp words",
            vec![set_text(var, "[light; dim]")],
            0,
            None,
        ),
    ];
    cmds.extend(more);
    cmds
}

fn with_lamp_var(var: &str, more: Vec<Vec<u8>>) -> BuiltMap {
    built(lamp_commands(var, more))
}

/// The profile variables of [`with_lamp_var`] are empty. A set whose name
/// pattern fits a variable of the request path leaves out each command
/// with a request write: the build then stops with too few records, which
/// also counts.
fn no_lamp_var(var: &str, more: Vec<Vec<u8>>) -> bool {
    match build_map(&vap(lamp_commands(var, more))) {
        Ok(b) => b.profile_vars.is_empty(),
        Err(e) => matches!(
            e,
            BuildError::Totals(Totals::TooFewRecords { found: 0, .. })
        ),
    }
}

/// A type 77 action (set a text from an integer) that writes `name`.
fn from_integer(name: &str) -> Vec<u8> {
    let mut f = action_fields(77);
    f[6] = str_field(name);
    record(f)
}

/// A command with no request write that sets the token-named variable
/// `name`, and the writers of the variables in the token.
fn harbour_loop(name: &str, writers: Vec<Vec<u8>>) -> Vec<u8> {
    let mut actions = writers;
    actions.push(set_text(name, "a harbour"));
    command("refresh the harbour list", actions, 0, None)
}

/// A set with a token in the variable name can set each variable whose
/// name fits its pattern. A `{TXT:V}` token that only type 77 writes gives
/// digits, an optional minus, or the text of a variable that is not set.
#[test]
fn test_a_token_with_digits_only_keeps_a_variable_with_letters_after_the_prefix() {
    let b = with_lamp_var(
        "LampWords",
        vec![harbour_loop(
            "Lamp{TXT:Counter}",
            vec![from_integer("Counter")],
        )],
    );
    assert_eq!(
        b.profile_vars,
        HashMap::from([("LampWords".to_string(), "[light; dim]".to_string())])
    );
}

#[test]
fn test_a_token_with_digits_only_removes_a_variable_of_the_prefix_and_digits() {
    for var in ["Lamp12", "LAMP-3", "lamp0", "Lampnot set"] {
        let b = with_lamp_var(
            var,
            vec![harbour_loop(
                "Lamp{TXT:Counter}",
                vec![from_integer("Counter")],
            )],
        );
        assert!(b.profile_vars.is_empty(), "{var}: {:?}", b.profile_vars);
    }
    // Literal text after the token must fit too.
    let pattern = vec![harbour_loop(
        "Lamp{TXT:Counter}Words",
        vec![from_integer("Counter")],
    )];
    assert!(with_lamp_var("Lamp7Words", pattern.clone())
        .profile_vars
        .is_empty());
    assert_eq!(with_lamp_var("LampWords", pattern).profile_vars.len(), 1);
}

#[test]
fn test_a_token_whose_variable_has_another_writer_removes_each_variable_of_the_prefix() {
    let other_writers = [
        vec![from_integer("Counter"), set_text("Counter", "x")],
        // No writer: the program cannot prove the form of the value.
        vec![],
        vec![
            from_integer("Counter"),
            inline_code("VA.SetText(\"counter\", x);"),
        ],
        vec![from_integer("Counter"), from_integer("{TXT:Other}")],
    ];
    for writers in other_writers {
        assert!(no_lamp_var(
            "LampWords",
            vec![harbour_loop("Lamp{TXT:Counter}", writers.clone())],
        ));
    }
    // A token of another kind can give any text.
    for name in [
        "Lamp{TXTNUM:Counter}",
        "Lamp{INT:Counter}",
        "Lamp{TXT:Counter",
    ] {
        let b = with_lamp_var(
            "LampWords",
            vec![harbour_loop(name, vec![from_integer("Counter")])],
        );
        assert!(b.profile_vars.is_empty(), "{name}: {:?}", b.profile_vars);
    }
    // A variable with another prefix stays.
    let b = with_lamp_var(
        "BuoyWords",
        vec![harbour_loop(
            "Lamp{TXT:Counter}",
            vec![set_text("Counter", "x")],
        )],
    );
    assert_eq!(b.profile_vars.len(), 1);
}

#[test]
fn test_a_name_that_is_one_token_only_removes_each_variable() {
    for name in ["{TXT:Other}", "{INT:Counter}"] {
        assert!(
            no_lamp_var("BuoyWords", vec![harbour_loop(name, vec![])]),
            "{name}"
        );
    }
    // A set of type 77 with a token in the name.
    assert!(no_lamp_var(
        "BuoyWords",
        vec![command(
            "count buoys",
            vec![from_integer("{TXT:Other}")],
            0,
            None,
        )],
    ));
    // One token that only type 77 writes gives digits: a name with letters
    // stays.
    let b = with_lamp_var(
        "BuoyWords",
        vec![harbour_loop("{TXT:Counter}", vec![from_integer("Counter")])],
    );
    assert_eq!(b.profile_vars.len(), 1);
}

/// A writer of a variable of the request path that is not a set-text: a
/// type 74 or 77 action, or a set with a token in the variable name whose
/// pattern fits the variable. The builder cannot know where VoiceAttack
/// writes the request, so each command that writes it is left out. Such a
/// writer in a command that a type 16 action runs counts too.
#[test]
fn test_another_writer_of_a_request_path_variable_leaves_each_writer_out() {
    let reason = "a variable of the request path has a writer that the program does not \
                  model, the program cannot know where VoiceAttack writes the request";
    let writer = |kind: i32, name: &str| {
        let mut f = action_fields(kind);
        f[6] = str_field(name);
        record(f)
    };
    // The two variables of the request path, in lowercase. The names
    // compare without regard to case.
    let vars = super::allow::write_path_vars();
    let (folder, file) = (vars[0].as_str(), vars[1].as_str());
    let file_upper = file.to_uppercase();
    // The pattern of the first 4 letters and any text fits the file
    // variable.
    let pattern = format!("{}{{TXT:Lantern}}", &file[..4]);
    let setups: Vec<Vec<Vec<u8>>> = vec![
        vec![command("other", vec![writer(77, file)], 0, None)],
        vec![command("other", vec![writer(74, folder)], 0, None)],
        vec![command(
            "other",
            vec![set_text("Lantern", "x"), set_text(&pattern, "x")],
            0,
            None,
        )],
        // The writer is in the target of an execute action.
        vec![
            command_with_id([3; 16], "keeper chores", vec![writer(77, &file_upper)]),
            command(
                "light the lighthouse",
                vec![write("Lighthouse:On"), exec_with([3; 16], None, false)],
                0,
                None,
            ),
        ],
    ];
    for (k, mut setup) in setups.into_iter().enumerate() {
        setup.push(flag_command(vec![write("BlueFlag:Up")]));
        let b = build_map(&deflate(&profile(setup))).unwrap_err();
        // Each record writes the request file: the map is too small.
        assert!(
            matches!(
                b,
                BuildError::Totals(Totals::TooFewRecords { found: 0, .. })
            ),
            "setup {k}: {b:?}"
        );
    }
    // With fewer writers: the reason of a left-out command.
    let mut cmds = vec![
        command("other", vec![writer(77, file)], 0, None),
        flag_command(vec![write("BlueFlag:Up")]),
    ];
    cmds.extend(filler(3));
    let out = super::extract::extract(&record::walk(&profile(cmds)).unwrap(), &[]).unwrap();
    assert!(out.commands.is_empty());
    let left: Vec<&str> = out
        .left_out
        .iter()
        .filter(|l| l.class == LeftOutClass::NotUnderstood)
        .map(|l| l.reason.as_str())
        .collect();
    assert_eq!(left, [reason; 4]);
    // A type 77 action of another variable changes nothing.
    let b = built(vec![
        command("other", vec![writer(77, &format!("{file}count"))], 0, None),
        flag_command(vec![write("BlueFlag:Up")]),
    ]);
    assert!(mapped(&b, "raise the blue flag"));
}

/// The record of `phrase`, or a panic with the reason of the left-out
/// command.
fn record_of_phrase<'a>(built: &'a BuiltMap, phrase: &str) -> &'a crate::Command {
    built
        .commands
        .iter()
        .find(|c| c.phrase.as_deref() == Some(phrase))
        .unwrap_or_else(|| panic!("{phrase:?} is not in the map: {:?}", left(built, phrase)))
}

/// A constant takes its value only from the one plain set-text before the
/// write. The names compare without regard to case. A set after the write
/// does not change the constant.
#[test]
fn test_constant_comes_from_the_one_set_before_the_write() {
    let sail = |phrase: &str, actions: Vec<Vec<u8>>| command(phrase, actions, 0, None);
    let token = "Sail:{TXT:SystemName}";
    let b = built(vec![
        // Another case before the write, the exact name after it.
        sail(
            "sail to the near quay",
            vec![
                set_text("systemname", "Near Quay"),
                write(token),
                set_text("SystemName", "Far Island"),
            ],
        ),
        // A value with a token first, the exact name after the write.
        sail(
            "sail to the lighthouse",
            vec![
                set_text("SYSTEMNAME", "{TXT:Elsewhere}"),
                write(token),
                set_text("SystemName", "Far Island"),
            ],
        ),
        // Two sets before the write.
        sail(
            "sail to the far island",
            vec![
                set_text("SystemName", "Near Quay"),
                set_text("SystemName", "Far Island"),
                write(token),
            ],
        ),
        // A set in a condition before the write.
        sail(
            "sail to the harbour",
            vec![
                condition("Tide", "high", 6),
                set_text("SystemName", "Near Quay"),
                end_condition(),
                write(token),
            ],
        ),
        // The same name before and after the write.
        sail(
            "sail to the old pier",
            vec![
                set_text("SystemName", "Old Pier"),
                write(token),
                set_text("SystemName", "Far Island"),
            ],
        ),
    ]);
    let constant = |phrase: &str| {
        record_of_phrase(&b, phrase)
            .constants
            .get("SystemName")
            .cloned()
    };
    assert_eq!(
        constant("sail to the near quay").as_deref(),
        Some("Near Quay")
    );
    assert_eq!(
        constant("sail to the old pier").as_deref(),
        Some("Old Pier")
    );
    for phrase in [
        "sail to the lighthouse",
        "sail to the far island",
        "sail to the harbour",
    ] {
        assert_eq!(
            not_understood(&b, phrase),
            "the command sets a part of its token in a way that the program does not model",
            "{phrase}"
        );
    }
}

/// A token that takes a phrase slot directly gets the first name of the
/// slot from the matcher. VoiceAttack writes the whole text of the slot
/// variable. The two are the same only for a slot kind whose file holds one
/// plain name (group, voice, point of interest).
#[test]
fn test_a_token_takes_a_phrase_slot_only_of_a_kind_with_one_plain_name() {
    let b = built(vec![
        command(
            "hail {TXT:Group2} at {TXT:System5}",
            vec![write("Hail:{TXT:Group2}:{TXT:System5}")],
            0,
            None,
        ),
        command(
            "go to {TXT:System5} now",
            vec![write("Go:{TXT:System5}:{CMD}")],
            0,
            None,
        ),
        command("open {TXT:Names}", vec![write("Open:{TXT:Names}")], 0, None),
        command(
            "tell {TXT:Voice1} about {TXT:Group2} near {TXT:POIType3}",
            vec![write("Tell:{TXT:Voice1}:{TXT:Group2}:{TXT:POIType3}")],
            0,
            None,
        ),
        // The slot is in the phrase, the token takes the system of the
        // slot parse: kept.
        command(
            "sail to {TXT:System5}",
            {
                let mut a = slot_parse(5);
                a.push(write("Sail:{TXT:SystemName}"));
                a
            },
            0,
            None,
        ),
    ]);
    let reason = "the token takes the whole text of a phrase slot whose file can hold more \
                  than one name";
    for phrase in [
        "hail {TXT:Group2} at {TXT:System5}",
        "go to {TXT:System5} now",
        "open {TXT:Names}",
    ] {
        assert_eq!(not_understood(&b, phrase), reason, "{phrase}");
    }
    assert!(mapped(
        &b,
        "tell {TXT:Voice1} about {TXT:Group2} near {TXT:POIType3}"
    ));
    assert!(mapped(&b, "sail to {TXT:System5}"));
}

/// A command that writes an empty text to the request file is left out
/// with a reason. The other commands build.
#[test]
fn test_an_empty_request_write_leaves_one_command_out() {
    let b = built(vec![
        command("clear the slate", vec![sound(), write("")], 0, None),
        flag_command(vec![write("BlueFlag:Up")]),
    ]);
    assert_eq!(
        not_understood(&b, "clear the slate"),
        "the command writes an empty text to the request file"
    );
    assert_eq!(left(&b, "clear the slate").unwrap().tokens, [""]);
    assert!(mapped(&b, "raise the blue flag"));
}

/// A command field of the length signature with another length is a
/// setting of one command: the builder leaves that command out. The id
/// (field 0) and the command type (field 19) still stop the build.
#[test]
fn test_another_length_of_a_command_setting_leaves_one_command_out() {
    let mut odd = command_fields("lower the red flag", vec![write("RedFlag:Down")], 0, None);
    // Field 9 has 4 bytes and more than one value in the tested profile:
    // only its length can mark it.
    odd[9] = vec![0; 8];
    let b = built(vec![record(odd), flag_command(vec![write("BlueFlag:Up")])]);
    assert_eq!(
        not_understood(&b, "lower the red flag"),
        "the command has a setting in field 9 that the program does not know"
    );
    assert!(mapped(&b, "raise the blue flag"));
    for field in [0, 19] {
        let mut odd = command_fields("lower the red flag", vec![write("RedFlag:Down")], 0, None);
        odd[field] = vec![1, 1];
        let err = build_map(&vap(vec![record(odd)])).unwrap_err();
        assert!(
            matches!(err, BuildError::Layout { field: f, .. } if f == field),
            "{err:?}"
        );
    }
}

/// Commands that each set a text and write it: `n` set-text actions with
/// `field` = `value`, and `plain` set-text actions with the tested value.
fn moods(n: usize, plain: usize, field: usize, value: Vec<u8>) -> Vec<Vec<u8>> {
    (0..n + plain)
        .map(|k| {
            let set = set_text("Mood", "calm");
            let set = if k < n {
                set_field(set, field, value.clone())
            } else {
                set
            };
            command(
                &format!("ring bell number {k}"),
                vec![set, write(&format!("Bell:{k}"))],
                0,
                None,
            )
        })
        .collect()
}

/// A tested field of one modelled action type with another value in more
/// than 20 percent of the actions of that type (and at least 50): the
/// format changed. The error names the action type and the field.
#[test]
fn test_a_field_change_of_many_actions_of_one_type_stops_the_build() {
    // 60 of 260 set-text actions: 23 percent of the type. Field 4 is not a
    // shared field.
    let e = build_map(&vap(moods(60, 200, 4, int_field(1)))).unwrap_err();
    assert_eq!(
        e,
        BuildError::FormatChanged {
            kind: RecordKind::Action,
            action_type: Some(21),
            field: 4,
            records: 60,
            total: 260
        }
    );
    assert_eq!(
        e.to_string(),
        "the format of the profile is not understood: 60 of 260 actions of type 21 have \
         another value in action field 4, the profile has a newer format than the tested \
         format, tell the developer"
    );
    // 60 of 400 (15 percent), and 49 of 100: only the commands leave.
    let b = built(moods(60, 340, 4, int_field(1)));
    assert_eq!(b.report.left_out_count(LeftOutClass::NotUnderstood), 60);
    let b = built(moods(49, 51, 4, int_field(1)));
    assert_eq!(b.report.left_out_count(LeftOutClass::NotUnderstood), 49);
    // Field 12 on each write of a profile.
    let cmds = (0..300)
        .map(|k| {
            command(
                &format!("gong {k}"),
                vec![set_field(write(&format!("G{k}")), 12, int_field(1))],
                0,
                None,
            )
        })
        .collect();
    let e = build_map(&deflate(&profile(cmds))).unwrap_err();
    assert!(
        matches!(
            e,
            BuildError::FormatChanged {
                action_type: Some(44),
                field: 12,
                records: 300,
                total: 300,
                ..
            }
        ),
        "{e:?}"
    );
}

/// A shared field also counts against the actions of each type: 60 sound
/// actions of 60 with a new value stop the build, although they are fewer
/// than 5 percent of all actions.
#[test]
fn test_a_shared_field_change_of_one_type_stops_the_build() {
    let mut cmds: Vec<Vec<u8>> = (0..60)
        .map(|k| {
            command(
                &format!("chime number {k}"),
                vec![
                    set_field(sound(), 25, vec![1; 16]),
                    write(&format!("Chime:{k}")),
                ],
                0,
                None,
            )
        })
        .collect();
    cmds.extend(filler(2000));
    let e = build_map(&deflate(&profile(cmds))).unwrap_err();
    assert_eq!(
        e,
        BuildError::FormatChanged {
            kind: RecordKind::Action,
            action_type: Some(14),
            field: 25,
            records: 60,
            total: 60
        }
    );
}

/// Commands with an action type that the program does not know before the
/// write, and commands without it.
fn new_type_commands(n: usize, plain: usize) -> Vec<Vec<u8>> {
    let mut cmds: Vec<Vec<u8>> = (0..n)
        .map(|k| {
            command(
                &format!("gong {k}"),
                vec![plain_action(80), write(&format!("G{k}"))],
                0,
                None,
            )
        })
        .collect();
    cmds.extend(
        (0..plain).map(|k| command(&format!("bell {k}"), vec![write(&format!("B{k}"))], 0, None)),
    );
    cmds
}

/// One reason that points at a change of the format leaves out more than
/// 25 percent of the commands with a request write: the build stops with a
/// typed error that names the reason and the count.
#[test]
fn test_a_format_reason_of_many_commands_stops_the_build() {
    let e = build_map(&deflate(&profile(new_type_commands(700, 300)))).unwrap_err();
    assert_eq!(
        e,
        BuildError::ReasonShare {
            reason: "action type 80 (not known) before the write".into(),
            commands: 700,
            total: 1000
        }
    );
    assert_eq!(
        e.to_string(),
        "the format of the profile is not understood: 700 of 1000 commands with a request \
         write are left out with the reason \"action type 80 (not known) before the write\", \
         the profile has a newer format than the tested format, tell the developer"
    );
    // 250 of 1000 is not more than 25 percent.
    let b = build_map(&deflate(&profile(new_type_commands(250, 750)))).unwrap();
    assert_eq!(b.report.left_out_count(LeftOutClass::NotUnderstood), 250);
    // A reason that does not point at the format: the build goes on.
    let mut cmds: Vec<Vec<u8>> = (0..700)
        .map(|k| {
            command(
                &format!("gong {k}"),
                vec![
                    set_text_copy("Tone", "Other"),
                    write(&format!("G{k}:{{TXT:Tone}}")),
                ],
                0,
                None,
            )
        })
        .collect();
    cmds.extend(filler(300));
    let b = build_map(&deflate(&profile(cmds))).unwrap();
    assert_eq!(b.report.left_out_count(LeftOutClass::NotUnderstood), 700);
}

/// The error of a map with too few records names the largest group of
/// left-out commands. A profile without a request write has its own
/// error.
#[test]
fn test_too_few_records_names_the_largest_reason() {
    let mut cmds = new_type_commands(30, 0);
    cmds.extend(filler(100));
    let e = build_map(&deflate(&profile(cmds))).unwrap_err();
    assert_eq!(
        e,
        BuildError::Totals(Totals::TooFewRecords {
            found: 100,
            minimum: 200,
            largest_reason: Some(("action type 80 (not known) before the write".into(), 30)),
        })
    );
    assert_eq!(
        e.to_string(),
        "the profile is not usable: the map has 100 commands, a profile of EDCoPilot has at \
         least 200, the largest group of left-out commands (30) has the reason \"action type \
         80 (not known) before the write\", tell the developer"
    );
    let cmds = (0..300)
        .map(|k| command(&format!("bell {k}"), vec![sound()], 0, None))
        .collect();
    let e = build_map(&deflate(&profile(cmds))).unwrap_err();
    assert_eq!(e, BuildError::Totals(Totals::NoRequestWrite));
    assert_eq!(
        e.to_string(),
        "the profile is not usable: no command of the profile writes a request to EDCoPilot, \
         tell the developer"
    );
}

/// The report counts the records with a phrase variable that is no slot,
/// no profile variable and no variable of the runtime grammar.
#[test]
fn test_the_report_counts_unknown_phrase_variables() {
    let b = built(vec![
        command("{TXT:Mystery} ahoy", vec![write("Ahoy")], 0, None),
        command("{TXT:Places} now", vec![write("Now")], 0, None),
        command("init", vec![set_text("Places", "[quay; pier]")], 0, None),
        command("{TXT:EDCP_HAIL} {TXT:Group3}", vec![write("Hail")], 0, None),
    ]);
    assert_eq!(b.report.unknown_phrase_vars, 1);
}

const TOKEN_PART: &str =
    "the command sets a part of its token in a way that the program does not model";

/// A set-text with another source than the literal (field 13, here 3) is
/// not plain: a constant from it is not modelled, and a prefix with it is
/// left out.
#[test]
fn test_a_set_text_with_another_source_is_not_plain() {
    let mut f = action_fields(21);
    f[5] = str_field("SystemName");
    f[6] = str_field("Far Island");
    f[13] = int_field(3);
    let source_3 = record(f);
    let mut f = action_fields(21);
    f[5] = str_field("Size");
    f[6] = str_field("Big:");
    f[13] = int_field(3);
    let prefix_source_3 = record(f);
    let b = built(vec![
        command(
            "sail to the far island",
            vec![source_3, write("Sail:{TXT:SystemName}")],
            0,
            None,
        ),
        command(
            "fly the [blue; red]",
            vec![prefix_source_3],
            1,
            Some("flags"),
        ),
    ]);
    assert_eq!(not_understood(&b, "sail to the far island"), TOKEN_PART);
    assert_eq!(
        not_understood(&b, "fly the [blue; red]"),
        "the prefix sets a variable in a way that the program does not model"
    );
}

/// The suffix of the known shape, with changes of its "has not been set"
/// block or of the field 2 of its blocks.
fn changed_suffix(change: impl Fn(&mut Vec<Vec<u8>>)) -> BuiltMap {
    let mut actions = vec![
        condition("{PREFIX}", "sell", 6),
        set_text("Side", "Sell"),
        end_condition(),
        condition("Side", "", 9),
        say("no"),
        else_branch(),
        write("{TXT:Side}Cork"),
        end_condition(),
    ];
    change(&mut actions);
    built(vec![
        command("sell", vec![set_text("Scope", "x")], 1, Some("market")),
        command("cork", actions, 2, Some("market")),
    ])
}

/// The "has not been set" block of a suffix needs the operator 9, a text
/// compare, the field 12 = 0, no compare value, and the field 2 = 0.0. A
/// `{PREFIX}` block needs the field 2 = 0.0 too.
#[test]
fn test_the_not_set_block_and_field_2_of_a_suffix_are_checked() {
    let unknown = "the suffix has a condition block of an unknown shape";
    let prefix_block = "the suffix has a {PREFIX} block of an unknown shape";
    assert!(mapped(&changed_suffix(|_| {}), "cork"));
    let b = changed_suffix(|a| a[3] = condition("Side", "", 6));
    assert_eq!(not_understood(&b, "cork"), unknown);
    let b = changed_suffix(|a| a[3] = set_field(a[3].clone(), 23, int_field(0)));
    assert_eq!(not_understood(&b, "cork"), unknown);
    let b = changed_suffix(|a| a[3] = set_field(a[3].clone(), 12, int_field(1)));
    assert_eq!(not_understood(&b, "cork"), unknown);
    let b = changed_suffix(|a| a[3] = condition("Side", "something", 9));
    assert_eq!(not_understood(&b, "cork"), unknown);
    let one = 1f64.to_le_bytes().to_vec();
    let b = changed_suffix(|a| a[3] = set_field(a[3].clone(), 2, one.clone()));
    assert_eq!(not_understood(&b, "cork"), unknown);
    let b = changed_suffix(|a| a[0] = set_field(a[0].clone(), 2, one.clone()));
    assert_eq!(not_understood(&b, "cork"), prefix_block);
}

/// The slot parse needs the tested fields 10, 11 and 19 of its four
/// set-integer actions, and no value (field 15) in the second of them.
#[test]
fn test_the_unread_fields_of_the_slot_parse_are_checked() {
    let phrase = "go to {TXT:System3}";
    let with = |at: usize, field: usize, value: Vec<u8>| {
        let mut actions = slot_parse(3);
        actions[at] = set_field(actions[at].clone(), field, value);
        actions.push(write("Go:{TXT:SystemName}"));
        built(vec![command(phrase, actions, 0, None)])
    };
    let mut actions = slot_parse(3);
    actions.push(write("Go:{TXT:SystemName}"));
    assert!(mapped(
        &built(vec![command(phrase, actions, 0, None)]),
        phrase
    ));
    // Without the slot parse, its sets of the token variable are not
    // modelled.
    for (at, field) in [(2, 10), (3, 11), (4, 19), (5, 10), (5, 11), (5, 19)] {
        let b = with(at, field, int_field(7));
        assert_eq!(
            not_understood(&b, phrase),
            TOKEN_PART,
            "action {at} field {field}"
        );
    }
    let b = with(3, 15, str_field("{INT:Startpos}"));
    assert_eq!(not_understood(&b, phrase), TOKEN_PART);
}

/// A target of an execute action with an action type that the table does
/// not list, or with a write or an execute action with an unknown setting,
/// can write a request token.
#[test]
fn test_a_target_with_an_unknown_type_or_setting_can_write() {
    let other_file = |name: &str| {
        let mut f = action_fields(44);
        f[5] = str_field("x");
        f[6] = str_field(&format!(r"{{TXT:PythonLibPath}}\{name}"));
        record(f)
    };
    let cmds = vec![
        command_with_id(
            [1; 16],
            "plain target",
            vec![other_file("EDCoPilot.VApath.txt")],
        ),
        command_with_id([2; 16], "odd type", vec![plain_action(99)]),
        command_with_id(
            [3; 16],
            "odd write",
            vec![set_field(
                other_file("EDCoPilot.VApath.txt"),
                11,
                int_field(1),
            )],
        ),
        command_with_id(
            [4; 16],
            "odd execute",
            vec![set_field(exec_by_id([1; 16]), 12, int_field(1))],
        ),
        command("to plain", vec![write("A"), exec_by_id([1; 16])], 0, None),
        command(
            "to odd type",
            vec![write("B"), exec_by_id([2; 16])],
            0,
            None,
        ),
        command(
            "to odd write",
            vec![write("C"), exec_by_id([3; 16])],
            0,
            None,
        ),
        command(
            "to odd execute",
            vec![write("D"), exec_by_id([4; 16])],
            0,
            None,
        ),
    ];
    let b = built(cmds);
    assert!(mapped(&b, "to plain"), "{:?}", left(&b, "to plain"));
    let writes = "action type 16 (execute a command) after the write: the command that it \
                  runs can write a request token";
    for phrase in ["to odd type", "to odd write", "to odd execute"] {
        assert_eq!(not_understood(&b, phrase), writes, "{phrase}");
    }
}

/// A condition block in a condition block, and an else-if, before the
/// write or in a prefix leave the command out.
#[test]
fn test_nested_conditions_and_else_if_before_the_write_are_left_out() {
    let nested = || {
        vec![
            condition("Tide", "high", 6),
            condition("Wind", "strong", 6),
            say("hold on"),
            end_condition(),
            end_condition(),
        ]
    };
    let else_if = || {
        vec![
            condition("Tide", "high", 6),
            say("high"),
            plain_action(63),
            say("low"),
            end_condition(),
        ]
    };
    let with_write = |mut a: Vec<Vec<u8>>| {
        a.push(write("Hatch:Open"));
        a
    };
    let with_set = |mut a: Vec<Vec<u8>>| {
        a.push(set_text("Size", "Big:"));
        a
    };
    let b = built(vec![
        command("open the hatch", with_write(nested()), 0, None),
        command("close the hatch", with_write(else_if()), 0, None),
        command("fly the [blue; red]", with_set(nested()), 1, Some("flags")),
        command(
            "fly the [green; white]",
            with_set(else_if()),
            1,
            Some("flags"),
        ),
    ]);
    assert_eq!(
        not_understood(&b, "open the hatch"),
        "a condition block before the write has an unknown shape"
    );
    assert_eq!(
        not_understood(&b, "close the hatch"),
        "action type 63 (else if) before the write"
    );
    assert_eq!(
        not_understood(&b, "fly the [blue; red]"),
        "a condition block of the prefix has an unknown shape"
    );
    assert_eq!(
        not_understood(&b, "fly the [green; white]"),
        "action type 63 (else if) in a prefix"
    );
}

/// A profile variable whose only set is switched off is not known.
#[test]
fn test_a_profile_variable_with_a_switched_off_set_is_not_known() {
    let b = built(vec![
        command("{TXT:Places} now", vec![write("P:{CMD}")], 0, None),
        command(
            "init",
            vec![flagged(set_text("Places", "[quay; pier]"))],
            0,
            None,
        ),
        command("{TXT:Spots} now", vec![write("Q:{CMD}")], 0, None),
        command("init two", vec![set_text("Spots", "[quay; pier]")], 0, None),
    ]);
    assert!(!b.profile_vars.contains_key("Places"));
    assert_eq!(
        b.profile_vars.get("Spots").map(String::as_str),
        Some("[quay; pier]")
    );
}

/// A newer format: a modelled action whose appended field has another
/// value than most actions of its type leaves its command out.
#[test]
fn test_an_odd_value_in_an_appended_field_leaves_the_command_out() {
    let b = with_extra_fields(1, || {
        let odd = set_field(set_text("N", "{TXTNUM:LastCMD}"), 33, vec![1, 0, 0, 0]);
        let cmds = vec![
            command(
                "lamp level [1..9]",
                vec![set_text("LastCMD", "{CMD}"), odd, write("L={TXT:N}")],
                0,
                None,
            ),
            command(
                "horn level [1..9]",
                vec![
                    set_text("LastCMD", "{CMD}"),
                    set_text("N", "{TXTNUM:LastCMD}"),
                    write("H={TXT:N}"),
                ],
                0,
                None,
            ),
        ];
        build_map(&vap(cmds)).unwrap()
    });
    assert_eq!(
        not_understood(&b, "lamp level [1..9]"),
        "an action has a setting that the program does not know"
    );
    assert!(mapped(&b, "horn level [1..9]"));
}

/// A condition block before the write has at most one `else`. A block
/// with two has no meaning that the program knows: the command is left
/// out. One `else` keeps the command.
#[test]
fn test_a_block_before_the_write_with_two_else_actions_is_not_understood() {
    let one = built(vec![flag_command(vec![
        condition("Wind", "", 9),
        say("calm"),
        else_branch(),
        say("windy"),
        end_condition(),
        write("BlueFlag:Up"),
    ])]);
    assert!(
        mapped(&one, "raise the blue flag"),
        "{:?}",
        one.report.left_out
    );
    let two = built(vec![flag_command(vec![
        condition("Wind", "", 9),
        say("calm"),
        else_branch(),
        say("windy"),
        else_branch(),
        say("stormy"),
        end_condition(),
        write("BlueFlag:Up"),
    ])]);
    assert!(!mapped(&two, "raise the blue flag"));
    assert_eq!(
        not_understood(&two, "raise the blue flag"),
        "a condition block before the write has an unknown shape"
    );
}
