//! Number readings on the synthetic profile: a run of single digits serves
//! only a name, and a range never takes a number with a leading zero.

mod common;

use common::{
    default_sound_alikes, fixture_commands_loaded, fixture_vocab, harbour_synonyms, token, writer,
};
use edcopilot_voicebridge_core::{AliasMap, Matcher, Pipeline};

/// The fixture with its vocabulary files (names with digits: `KTL 3081`,
/// `17 Skerry`, `Reef 2207`), the default sound-alikes and the invented
/// synonym table.
fn pipeline() -> Pipeline {
    let matcher = Matcher::new(
        fixture_commands_loaded(),
        fixture_vocab(),
        AliasMap::new(),
        0.8,
    );
    Pipeline::new(matcher, writer())
        .with_sound_alikes(default_sound_alikes())
        .with_synonyms(harbour_synonyms())
}

/// A digit run that lands in a name works: a vocabulary name with the
/// digits after a word, before a word, and a fixed system name, also after
/// a profile variable.
#[test]
fn test_digit_run_in_a_name() {
    let p = pipeline();
    for (text, expected) in [
        (
            "sail to reef two two zero seven",
            "HarbourAt:Reef 2207:sail to",
        ),
        (
            "sail to ktl three zero eight one",
            "HarbourAt:KTL 3081:sail to",
        ),
        ("sail to one seven skerry", "HarbourAt:17 Skerry:sail to"),
        (
            "sail to skerry four zero nine",
            "HarbourAt:Skerry 409:sail to",
        ),
        (
            "tide at skerry four zero nine",
            "HarbourInfo:Skerry 409:tide at Skerry 409",
        ),
        // A profile variable and a slot name with a digit run.
        (
            "tide at ktl three zero eight one",
            "HarbourInfo:KTL 3081:tide at KTL 3081",
        ),
        (
            "depth at reef two two zero seven",
            "HarbourInfo:Reef 2207:depth at Reef 2207",
        ),
    ] {
        assert_eq!(token(&p, text).as_deref(), Some(expected), "{text}");
    }
}

/// A digit run never becomes a value of a range or a literal number.
#[test]
fn test_digit_run_is_no_value() {
    let p = pipeline();
    for text in [
        "fog horn volume five five",
        "fog horn volume 5 5",
        "hoist pennant one two",
        "remove crate 1 2 3",
    ] {
        assert_eq!(token(&p, text), None, "{text}");
    }
}

/// The readings that worked before stay: a scaled range, two ranges for
/// one value (also after a synonym), and an optional leading zero.
#[test]
fn test_base_number_readings_stay() {
    let p = pipeline();
    for (text, expected) in [
        ("fog horn volume fifty five", "Prefs:HornVolume=\"55\""),
        ("fog horn volume one hundred", "Prefs:HornVolume=\"100\""),
        ("make for buoy three four", "SailForBuoy:34"),
        ("pick mooring three four", "PickMooringVoice:34"),
        ("mooring zero five", "SelectMooring:05"),
    ] {
        assert_eq!(token(&p, text).as_deref(), Some(expected), "{text}");
    }
}

/// A range takes no number with a leading zero.
#[test]
fn test_range_refuses_a_leading_zero() {
    let p = pipeline();
    for text in [
        "fog horn volume zero five",
        "fog horn volume 0 5",
        "fog horn volume 05",
        "net reminder zero five zero",
        "hoist pennant zero zero five",
    ] {
        assert_eq!(token(&p, text), None, "{text}");
    }
}
