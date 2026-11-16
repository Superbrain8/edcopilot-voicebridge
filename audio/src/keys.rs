//! Talk key specifications: text to a Windows virtual-key code, and back.
//!
//! The functions here are pure. The layout look-ups (`VkKeyScanW`,
//! `MapVirtualKeyW`) come in as closures, so that tests can give a fake
//! keyboard layout.

/// A parsed key specification, before the keyboard layout is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeySpec {
    /// A canonical key name, for example `F13` or `Mouse4`.
    Name(&'static str),
    /// One printable character. The active keyboard layout gives the key.
    Char(char),
    /// A raw virtual-key code from 1 to 254.
    Vk(u8),
}

/// The key that types one character on a keyboard layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutKey {
    pub vk: u8,
    /// The modifiers that the character needs, as `VkKeyScanW` gives them:
    /// 1 Shift, 2 Ctrl, 4 Alt (AltGr is Ctrl and Alt), other bits for other
    /// states. 0 means no modifier.
    pub shift_state: u8,
}

/// The names and their Windows virtual-key codes. For a code with two
/// names, the first name is the one that `inputs` prints.
const NAMED_KEYS: &[(&str, u8)] = &[
    ("Key0", 0x30),
    ("Key1", 0x31),
    ("Key2", 0x32),
    ("Key3", 0x33),
    ("Key4", 0x34),
    ("Key5", 0x35),
    ("Key6", 0x36),
    ("Key7", 0x37),
    ("Key8", 0x38),
    ("Key9", 0x39),
    ("A", 0x41),
    ("B", 0x42),
    ("C", 0x43),
    ("D", 0x44),
    ("E", 0x45),
    ("F", 0x46),
    ("G", 0x47),
    ("H", 0x48),
    ("I", 0x49),
    ("J", 0x4A),
    ("K", 0x4B),
    ("L", 0x4C),
    ("M", 0x4D),
    ("N", 0x4E),
    ("O", 0x4F),
    ("P", 0x50),
    ("Q", 0x51),
    ("R", 0x52),
    ("S", 0x53),
    ("T", 0x54),
    ("U", 0x55),
    ("V", 0x56),
    ("W", 0x57),
    ("X", 0x58),
    ("Y", 0x59),
    ("Z", 0x5A),
    ("F1", 0x70),
    ("F2", 0x71),
    ("F3", 0x72),
    ("F4", 0x73),
    ("F5", 0x74),
    ("F6", 0x75),
    ("F7", 0x76),
    ("F8", 0x77),
    ("F9", 0x78),
    ("F10", 0x79),
    ("F11", 0x7A),
    ("F12", 0x7B),
    ("F13", 0x7C),
    ("F14", 0x7D),
    ("F15", 0x7E),
    ("F16", 0x7F),
    ("F17", 0x80),
    ("F18", 0x81),
    ("F19", 0x82),
    ("F20", 0x83),
    ("Escape", 0x1B),
    ("Space", 0x20),
    ("LControl", 0xA2),
    ("RControl", 0xA3),
    ("LShift", 0xA0),
    ("RShift", 0xA1),
    ("LAlt", 0xA4),
    ("RAlt", 0xA5),
    ("LMeta", 0x5B),
    ("RMeta", 0x5C),
    // macOS names of the same keys.
    ("Command", 0x5B),
    ("RCommand", 0x5C),
    ("LOption", 0xA4),
    ("ROption", 0xA5),
    ("Enter", 0x0D),
    ("Up", 0x26),
    ("Down", 0x28),
    ("Left", 0x25),
    ("Right", 0x27),
    ("Backspace", 0x08),
    ("CapsLock", 0x14),
    ("Tab", 0x09),
    ("Home", 0x24),
    ("End", 0x23),
    ("PageUp", 0x21),
    ("PageDown", 0x22),
    ("Insert", 0x2D),
    ("Delete", 0x2E),
    ("Numpad0", 0x60),
    ("Numpad1", 0x61),
    ("Numpad2", 0x62),
    ("Numpad3", 0x63),
    ("Numpad4", 0x64),
    ("Numpad5", 0x65),
    ("Numpad6", 0x66),
    ("Numpad7", 0x67),
    ("Numpad8", 0x68),
    ("Numpad9", 0x69),
    ("NumpadSubtract", 0x6D),
    ("NumpadAdd", 0x6B),
    ("NumpadDivide", 0x6F),
    ("NumpadMultiply", 0x6A),
    ("NumpadEquals", 0x92),
    ("NumpadDecimal", 0x6E),
    ("Grave", 0xC0),
    ("Minus", 0xBD),
    ("Equal", 0xBB),
    ("LeftBracket", 0xDB),
    ("RightBracket", 0xDD),
    ("BackSlash", 0xDC),
    ("Semicolon", 0xBA),
    ("Apostrophe", 0xDE),
    ("Comma", 0xBC),
    ("Dot", 0xBE),
    ("Slash", 0xBF),
    ("Mouse4", 0x05),
    ("Mouse5", 0x06),
];

/// A `device_query` name that has no separate code on Windows: the numpad
/// Enter key gives the same code as the main Enter key.
const NUMPAD_ENTER: &str = "NumpadEnter";

/// Names of the main mouse buttons. They are refused with a reason.
const MAIN_MOUSE_NAMES: &[&str] = &[
    "Mouse1",
    "Mouse2",
    "Mouse3",
    "MouseLeft",
    "MouseRight",
    "MouseMiddle",
    "LButton",
    "RButton",
    "MButton",
];

/// Virtual-key codes of the left, right and middle mouse buttons.
const MAIN_MOUSE_VKS: &[u8] = &[0x01, 0x02, 0x04];

/// Generic Shift, Control and Alt. Windows reports them together with the
/// left or right code.
const GENERIC_MODIFIER_VKS: &[u8] = &[0x10, 0x11, 0x12];

fn main_mouse_error(spec: &str) -> String {
    format!(
        "{spec:?} is a main mouse button: the left, right and middle mouse buttons cannot be \
         the talk key, because the game and the desktop need them (use Mouse4 or Mouse5)"
    )
}

fn unknown_error(spec: &str) -> String {
    format!(
        "unknown key {spec:?}. Use a key name (for example F13, RControl, Space, A, Key1, \
         Numpad0), one character of your keyboard (for example \"<\", \"ö\", \"#\"), a \
         virtual-key code (for example vk:226 or vk:0xE2), or Mouse4 or Mouse5. Run \
         \"voicebridge inputs\" and press the key to see its name"
    )
}

/// Parses a key specification. Names and `vk:` are not case-sensitive.
/// White space at the start and at the end is ignored.
pub fn parse_key_spec(spec: &str) -> Result<KeySpec, String> {
    let text = spec.trim();
    if text.is_empty() {
        return Err(unknown_error(spec));
    }
    if text.get(..3).is_some_and(|p| p.eq_ignore_ascii_case("vk:")) {
        return parse_vk(text, &text[3..]);
    }
    if let Some((name, _)) = NAMED_KEYS
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(text))
    {
        return Ok(KeySpec::Name(name));
    }
    if NUMPAD_ENTER.eq_ignore_ascii_case(text) {
        return Ok(KeySpec::Name(NUMPAD_ENTER));
    }
    if MAIN_MOUSE_NAMES
        .iter()
        .any(|n| n.eq_ignore_ascii_case(text))
    {
        return Err(main_mouse_error(text));
    }
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_control() => {
            Err(format!("{spec:?} is a control character, not a key"))
        }
        (Some(c), None) => Ok(KeySpec::Char(c)),
        _ => Err(unknown_error(spec)),
    }
}

/// Parses the number part of `vk:<number>`, decimal or `0x` hex.
fn parse_vk(spec: &str, number: &str) -> Result<KeySpec, String> {
    let parsed = match number.get(..2) {
        Some(prefix) if prefix.eq_ignore_ascii_case("0x") => {
            u32::from_str_radix(&number[2..], 16).ok()
        }
        _ => number.parse::<u32>().ok(),
    };
    let vk = parsed.filter(|n| (1..=254).contains(n)).ok_or_else(|| {
        format!(
            "{spec:?} is not a valid virtual-key code: write vk:<number> with a number \
                 from 1 to 254, decimal (vk:226) or hex (vk:0xE2)"
        )
    })? as u8;
    if MAIN_MOUSE_VKS.contains(&vk) {
        return Err(main_mouse_error(spec));
    }
    Ok(KeySpec::Vk(vk))
}

/// The virtual-key code of a canonical name. `None` for a name that has no
/// separate code on Windows.
pub fn vk_of_name(name: &str) -> Option<u8> {
    NAMED_KEYS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, vk)| *vk)
}

/// The preferred name of a virtual-key code, if it has one.
pub fn name_of_vk(vk: u8) -> Option<&'static str> {
    NAMED_KEYS
        .iter()
        .find(|(_, code)| *code == vk)
        .map(|(name, _)| *name)
}

/// The virtual-key code of a parsed specification. `vk_of_char` is the
/// layout look-up for one character.
pub fn resolve_vk(
    spec: KeySpec,
    vk_of_char: impl Fn(char) -> Option<LayoutKey>,
) -> Result<u8, String> {
    match spec {
        KeySpec::Name(name) => vk_of_name(name).ok_or_else(|| {
            format!(
                "{name} cannot be told apart from Enter on Windows, both keys give the same \
                 code. Use Enter or another key"
            )
        }),
        KeySpec::Char(c) => {
            let key = vk_of_char(c).ok_or_else(|| {
                format!(
                    "the active keyboard layout cannot produce the character {c:?}. Run \
                     \"voicebridge bind\" or \"voicebridge inputs\" and press the key"
                )
            })?;
            if key.shift_state != 0 {
                // The talk key would be another key than the one that types `c`.
                return Err(format!(
                    "the character {c:?} needs a modifier (Shift, Ctrl, Alt or AltGr) on the \
                     active keyboard layout. Run \"voicebridge bind\" and press the key, or \
                     write the character without the modifier, or write vk:{}",
                    key.vk
                ));
            }
            Ok(key.vk)
        }
        KeySpec::Vk(vk) => Ok(vk),
    }
}

/// `true` for the punctuation keys. Their names give the US position, so
/// the character of the active layout is clearer.
fn is_punctuation_vk(vk: u8) -> bool {
    matches!(vk, 0xBA..=0xC0 | 0xDB..=0xDF | 0xE1..=0xE4)
}

/// The text that `inputs` prints for a pressed key. It parses back to `vk`.
///
/// For a punctuation key the order is: the character of the key on the
/// layout, the known name, `vk:<decimal>`. For other keys the known name
/// comes first. A character is used only if it types without a modifier and
/// gives the same code back.
pub fn spec_for_vk(
    vk: u8,
    char_of_vk: impl Fn(u8) -> Option<char>,
    vk_of_char: impl Fn(char) -> Option<LayoutKey>,
) -> String {
    let name = name_of_vk(vk);
    if let Some(name) = name.filter(|_| !is_punctuation_vk(vk)) {
        return name.to_string();
    }
    if let Some(text) = layout_char_spec(vk, char_of_vk, vk_of_char) {
        return text;
    }
    match name {
        Some(name) => name.to_string(),
        None => format!("vk:{vk}"),
    }
}

/// The character of `vk` on the layout, if it parses back to `vk`.
fn layout_char_spec(
    vk: u8,
    char_of_vk: impl Fn(u8) -> Option<char>,
    vk_of_char: impl Fn(char) -> Option<LayoutKey>,
) -> Option<String> {
    if let Some(c) = char_of_vk(vk) {
        let mut candidates: Vec<char> = Vec::new();
        let mut lower = c.to_lowercase();
        if let (Some(l), None) = (lower.next(), lower.next()) {
            candidates.push(l);
        }
        candidates.push(c);
        for c in candidates {
            // A quote or a backslash needs escapes in TOML.
            if c.is_whitespace() || c == '"' || c == '\\' {
                continue;
            }
            let text = c.to_string();
            let back = parse_key_spec(&text).and_then(|spec| resolve_vk(spec, &vk_of_char));
            if back == Ok(vk) {
                return Some(text);
            }
        }
    }
    None
}

/// `false` for the codes that `inputs` does not print: the left, right and
/// middle mouse buttons, and the generic Shift, Control and Alt codes.
pub fn is_reported_vk(vk: u8) -> bool {
    (1..=254).contains(&vk) && !MAIN_MOUSE_VKS.contains(&vk) && !GENERIC_MODIFIER_VKS.contains(&vk)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHIFT: u8 = 1;
    const ALT_GR: u8 = 6;

    /// A fake German layout: `<` is VK_OEM_102, `ö` is VK_OEM_3, `#` is
    /// VK_OEM_2, `+` is VK_OEM_PLUS, `@` is AltGr+Q.
    fn german_vk_of_char(c: char) -> Option<LayoutKey> {
        let (vk, shift_state) = match c {
            '<' => (0xE2, 0),
            '>' => (0xE2, SHIFT),
            'ö' => (0xC0, 0),
            'Ö' => (0xC0, SHIFT),
            '#' => (0xBF, 0),
            '+' => (0xBB, 0),
            '1' => (0x31, 0),
            'a' => (0x41, 0),
            'A' => (0x41, SHIFT),
            '@' => (0x51, ALT_GR),
            // A fake key without a name, with an upper case character.
            'ä' => (0xE4, 0),
            'Ä' => (0xE4, SHIFT),
            // A fake key that types its character only with Shift.
            '§' => (0xDF, SHIFT),
            _ => return None,
        };
        Some(LayoutKey { vk, shift_state })
    }

    fn german_char_of_vk(vk: u8) -> Option<char> {
        match vk {
            0xE2 => Some('<'),
            0xC0 => Some('Ö'),
            0xBF => Some('#'),
            0xBB => Some('+'),
            0x41 => Some('A'),
            0xE4 => Some('Ä'),
            0xDF => Some('§'),
            // A key whose character the layout gives to another key.
            0xE1 => Some('<'),
            0xE3 => Some('"'),
            _ => None,
        }
    }

    #[test]
    fn test_names_are_not_case_sensitive() {
        assert_eq!(parse_key_spec("F13"), Ok(KeySpec::Name("F13")));
        assert_eq!(parse_key_spec("f13"), Ok(KeySpec::Name("F13")));
        assert_eq!(parse_key_spec("rcontrol"), Ok(KeySpec::Name("RControl")));
        assert_eq!(parse_key_spec("a"), Ok(KeySpec::Name("A")));
        assert_eq!(parse_key_spec("mouse4"), Ok(KeySpec::Name("Mouse4")));
        assert_eq!(parse_key_spec("  Space "), Ok(KeySpec::Name("Space")));
    }

    #[test]
    fn test_known_names_map_to_virtual_keys() {
        let expected = [
            ("F13", 0x7C),
            ("F1", 0x70),
            ("F20", 0x83),
            ("RControl", 0xA3),
            ("LShift", 0xA0),
            ("RAlt", 0xA5),
            ("Space", 0x20),
            ("A", 0x41),
            ("Z", 0x5A),
            ("Key1", 0x31),
            ("Numpad0", 0x60),
            ("NumpadAdd", 0x6B),
            ("Grave", 0xC0),
            ("Slash", 0xBF),
            ("Escape", 0x1B),
            ("Enter", 0x0D),
            ("PageDown", 0x22),
            ("Mouse4", 0x05),
            ("Mouse5", 0x06),
        ];
        for (name, vk) in expected {
            assert_eq!(vk_of_name(name), Some(vk), "{name}");
        }
    }

    #[test]
    fn test_every_device_query_name_is_still_accepted() {
        for name in DEVICE_QUERY_NAMES {
            let spec = parse_key_spec(name).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(spec, KeySpec::Name(name));
            let lower = parse_key_spec(&name.to_lowercase()).unwrap();
            assert_eq!(lower, KeySpec::Name(name));
        }
    }

    #[test]
    fn test_every_name_but_numpad_enter_has_a_virtual_key() {
        for name in DEVICE_QUERY_NAMES.iter().chain(&["Mouse4", "Mouse5"]) {
            if *name == "NumpadEnter" {
                assert_eq!(vk_of_name(name), None);
            } else {
                assert!(vk_of_name(name).is_some(), "{name}");
            }
        }
    }

    #[test]
    fn test_numpad_enter_is_an_error_on_resolve() {
        let spec = parse_key_spec("NumpadEnter").unwrap();
        let err = resolve_vk(spec, german_vk_of_char).unwrap_err();
        assert!(err.contains("Enter"), "{err}");
    }

    #[test]
    fn test_single_characters() {
        assert_eq!(parse_key_spec("<"), Ok(KeySpec::Char('<')));
        assert_eq!(parse_key_spec("ö"), Ok(KeySpec::Char('ö')));
        assert_eq!(parse_key_spec("#"), Ok(KeySpec::Char('#')));
        assert_eq!(parse_key_spec("+"), Ok(KeySpec::Char('+')));
        assert_eq!(parse_key_spec("1"), Ok(KeySpec::Char('1')));
        assert_eq!(resolve_vk(KeySpec::Char('<'), german_vk_of_char), Ok(0xE2));
        assert_eq!(resolve_vk(KeySpec::Char('ö'), german_vk_of_char), Ok(0xC0));
    }

    #[test]
    fn test_character_that_needs_a_modifier_is_refused() {
        for c in ['>', 'Ö', 'A', '@', '§'] {
            let err = resolve_vk(KeySpec::Char(c), german_vk_of_char).unwrap_err();
            assert!(err.contains(c), "{c}: {err}");
            assert!(err.contains("modifier"), "{c}: {err}");
            assert!(err.contains("voicebridge bind"), "{c}: {err}");
            assert!(err.contains("vk:"), "{c}: {err}");
        }
    }

    #[test]
    fn test_character_the_layout_cannot_produce() {
        let err = resolve_vk(KeySpec::Char('ł'), german_vk_of_char).unwrap_err();
        assert!(err.contains("layout"), "{err}");
        assert!(err.contains('ł'), "{err}");
    }

    #[test]
    fn test_control_character_is_an_error() {
        assert!(parse_key_spec("\u{7}").is_err());
    }

    #[test]
    fn test_raw_virtual_key_codes() {
        assert_eq!(parse_key_spec("vk:226"), Ok(KeySpec::Vk(226)));
        assert_eq!(parse_key_spec("VK:0xE2"), Ok(KeySpec::Vk(0xE2)));
        assert_eq!(parse_key_spec("vk:0xe2"), Ok(KeySpec::Vk(0xE2)));
        assert_eq!(parse_key_spec("vk:254"), Ok(KeySpec::Vk(254)));
        assert_eq!(parse_key_spec("vk:5"), Ok(KeySpec::Vk(5)));
        for bad in [
            "vk:0", "vk:255", "vk:0x100", "vk:", "vk:-1", "vk:abc", "vk:1x",
        ] {
            let err = parse_key_spec(bad).unwrap_err();
            assert!(err.contains("vk:"), "{bad}: {err}");
        }
        assert_eq!(resolve_vk(KeySpec::Vk(226), german_vk_of_char), Ok(226));
    }

    #[test]
    fn test_main_mouse_buttons_are_refused() {
        for bad in [
            "vk:1",
            "vk:2",
            "vk:4",
            "vk:0x01",
            "Mouse1",
            "mouse2",
            "Mouse3",
            "MouseLeft",
            "MouseRight",
            "MouseMiddle",
            "LButton",
            "RButton",
            "MButton",
        ] {
            let err = parse_key_spec(bad).unwrap_err();
            assert!(err.contains("mouse"), "{bad}: {err}");
            assert!(err.contains("game"), "{bad}: {err}");
        }
    }

    #[test]
    fn test_unknown_spec_names_all_forms() {
        for bad in ["NoSuchKey", "", "   ", "13", "<<", "öö"] {
            let err = parse_key_spec(bad).unwrap_err();
            assert!(err.contains("F13"), "{bad}: {err}");
            assert!(err.contains("vk:226"), "{bad}: {err}");
            assert!(err.contains("Mouse4"), "{bad}: {err}");
            assert!(err.contains("\"<\""), "{bad}: {err}");
            assert!(err.contains("voicebridge inputs"), "{bad}: {err}");
        }
        assert!(parse_key_spec("NoSuchKey")
            .unwrap_err()
            .contains("NoSuchKey"));
    }

    #[test]
    fn test_known_names_round_trip() {
        for vk in 1..=254u8 {
            if let Some(name) = name_of_vk(vk) {
                let spec = spec_for_vk(vk, |_| None, |_| None);
                assert_eq!(spec, name);
                let back = resolve_vk(parse_key_spec(&spec).unwrap(), |_| None);
                assert_eq!(back, Ok(vk), "{spec}");
            }
        }
        for name in DEVICE_QUERY_NAMES.iter().chain(&["Mouse4", "Mouse5"]) {
            let Some(vk) = vk_of_name(name) else { continue };
            let spec = spec_for_vk(vk, |_| None, |_| None);
            let back = resolve_vk(parse_key_spec(&spec).unwrap(), |_| None);
            assert_eq!(back, Ok(vk), "{name} printed as {spec}");
        }
    }

    #[test]
    fn test_preferred_names() {
        assert_eq!(name_of_vk(0x5B), Some("LMeta"));
        assert_eq!(name_of_vk(0xA4), Some("LAlt"));
        assert_eq!(name_of_vk(0x05), Some("Mouse4"));
        assert_eq!(name_of_vk(0xE2), None);
    }

    #[test]
    fn test_raw_codes_round_trip() {
        for vk in (1..=254u8).filter(|vk| is_reported_vk(*vk)) {
            if name_of_vk(vk).is_some() {
                continue;
            }
            let spec = spec_for_vk(vk, |_| None, |_| None);
            assert_eq!(spec, format!("vk:{vk}"));
            let back = resolve_vk(parse_key_spec(&spec).unwrap(), |_| None);
            assert_eq!(back, Ok(vk), "{spec}");
        }
    }

    #[test]
    fn test_layout_characters_round_trip() {
        let spec = |vk| spec_for_vk(vk, german_char_of_vk, german_vk_of_char);
        // Punctuation keys print the character of the layout first.
        assert_eq!(spec(0xE2), "<");
        assert_eq!(spec(0xBF), "#");
        assert_eq!(spec(0xC0), "ö");
        assert_eq!(spec(0xBB), "+");
        // The layout gives `Ä`, the lower case form types without Shift.
        assert_eq!(spec(0xE4), "ä");
        // Other keys print the name first.
        assert_eq!(spec(0x41), "A");
        // A punctuation key without a character prints its name.
        assert_eq!(spec(0xDC), "BackSlash");
        // A character that needs Shift is never printed.
        assert_eq!(spec(0xDF), "vk:223");
        // The layout gives `<` to VK_OEM_102, so 0xE1 needs the raw code.
        assert_eq!(spec(0xE1), "vk:225");
        // A quote needs escapes in TOML, so it is printed as a raw code.
        assert_eq!(spec(0xE3), "vk:227");
        for vk in [0x41, 0xBB, 0xBF, 0xC0, 0xDC, 0xDF, 0xE1, 0xE2, 0xE3, 0xE4] {
            let text = spec(vk);
            let back = resolve_vk(parse_key_spec(&text).unwrap(), german_vk_of_char);
            assert_eq!(back, Ok(vk), "{text}");
        }
    }

    #[test]
    fn test_reported_codes() {
        for vk in [1, 2, 4, 0x10, 0x11, 0x12] {
            assert!(!is_reported_vk(vk), "{vk}");
        }
        for vk in [5, 6, 0x0D, 0xA0, 0xA3, 0xE2, 0x7C] {
            assert!(is_reported_vk(vk), "{vk}");
        }
    }

    /// Every name that `device_query::Keycode::from_str` accepts.
    const DEVICE_QUERY_NAMES: &[&str] = &[
        "Key0",
        "Key1",
        "Key2",
        "Key3",
        "Key4",
        "Key5",
        "Key6",
        "Key7",
        "Key8",
        "Key9",
        "A",
        "B",
        "C",
        "D",
        "E",
        "F",
        "G",
        "H",
        "I",
        "J",
        "K",
        "L",
        "M",
        "N",
        "O",
        "P",
        "Q",
        "R",
        "S",
        "T",
        "U",
        "V",
        "W",
        "X",
        "Y",
        "Z",
        "F1",
        "F2",
        "F3",
        "F4",
        "F5",
        "F6",
        "F7",
        "F8",
        "F9",
        "F10",
        "F11",
        "F12",
        "F13",
        "F14",
        "F15",
        "F16",
        "F17",
        "F18",
        "F19",
        "F20",
        "Escape",
        "Space",
        "LControl",
        "RControl",
        "LShift",
        "RShift",
        "LAlt",
        "RAlt",
        "Command",
        "RCommand",
        "LOption",
        "ROption",
        "LMeta",
        "RMeta",
        "Enter",
        "Up",
        "Down",
        "Left",
        "Right",
        "Backspace",
        "CapsLock",
        "Tab",
        "Home",
        "End",
        "PageUp",
        "PageDown",
        "Insert",
        "Delete",
        "Numpad0",
        "Numpad1",
        "Numpad2",
        "Numpad3",
        "Numpad4",
        "Numpad5",
        "Numpad6",
        "Numpad7",
        "Numpad8",
        "Numpad9",
        "NumpadSubtract",
        "NumpadAdd",
        "NumpadDivide",
        "NumpadMultiply",
        "NumpadEquals",
        "NumpadEnter",
        "NumpadDecimal",
        "Grave",
        "Minus",
        "Equal",
        "LeftBracket",
        "RightBracket",
        "BackSlash",
        "Semicolon",
        "Apostrophe",
        "Comma",
        "Dot",
        "Slash",
    ];
}
