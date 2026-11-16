//! Spoken numbers to digits. Speech-to-text can return number words where the
//! grammar needs digits.

use crate::matcher::normalize;

/// Largest number this module writes as digits. A phrase above it stays as words.
const MAX_NUMBER: u64 = 9_999_999;

/// Punctuation that speech-to-text puts after a word.
const TRAILING_PUNCT: [char; 6] = [',', '.', ';', ':', '!', '?'];

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    /// 0 to 19
    Unit(u64),
    /// 20, 30, ... 90
    Tens(u64),
    Hundred,
    Thousand,
    Million,
    And,
    /// English "a" before "hundred" or "thousand".
    A,
    /// A German one-word number. The flag is true when it contains a scale word.
    Compound(u64, bool),
}

struct Token<'a> {
    word: &'a str,
    kind: Option<Kind>,
    /// Digits for a digit group such as `5,000`.
    digits: Option<String>,
    /// For a digit group or a plain number that is a whole number of
    /// thousands (1000 or more): the count of thousands.
    thousands: Option<u64>,
    /// The word ended with punctuation. A number cannot continue past it.
    punct: bool,
}

/// How `render` writes the numbers of one variant.
#[derive(Clone, Copy, PartialEq)]
enum Pass {
    /// Each number phrase as one number.
    Full,
    /// Only the number words below one hundred.
    Units,
    /// As `Full`, and whole thousands in digits as "N thousand".
    Thousands,
}

/// Return candidate texts to try after the original text: first every number
/// phrase as one number, then only the number words below one hundred, then
/// whole thousands in digits as "N thousand", then each run of two or more
/// single digits as one number ("one zero eight two eight" as "10828"). The
/// result has no duplicates and never holds the original text.
pub fn number_variants(text: &str) -> Vec<String> {
    number_readings(text).into_iter().map(|r| r.text).collect()
}

/// One number reading of a text.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct NumberReading {
    pub text: String,
    /// For a digit-run reading: the positions of the joined numbers in the
    /// normalised words of `text` (see `matcher::normalize`). Empty for the
    /// other readings.
    pub digit_runs: Vec<usize>,
}

/// As `number_variants`, with the positions of the joined digit runs.
pub(crate) fn number_readings(text: &str) -> Vec<NumberReading> {
    let text_split = split_hyphenated(text);
    let tokens = tokenize(&text_split);
    let full_kinds: Vec<Option<Kind>> = (0..tokens.len())
        .map(|i| match tokens[i].kind {
            Some(Kind::A) => Some(Kind::Unit(1)),
            other => other,
        })
        .collect();
    let unit_kinds: Vec<Option<Kind>> = tokens.iter().map(|t| t.kind).collect();
    let seg_end = segment_ends(&tokens);

    let mut variants = Vec::new();
    for pass in [Pass::Full, Pass::Units, Pass::Thousands] {
        let (candidate, changed) = render(&tokens, &full_kinds, &unit_kinds, &seg_end, pass);
        if changed
            && candidate != text
            && !variants.iter().any(|r: &NumberReading| r.text == candidate)
        {
            variants.push(NumberReading {
                text: candidate,
                digit_runs: Vec::new(),
            });
        }
    }
    if let Some((candidate, positions)) = digit_runs_with_positions(&text_split) {
        if candidate != text && !variants.iter().any(|r| r.text == candidate) {
            variants.push(NumberReading {
                text: candidate,
                digit_runs: positions,
            });
        }
    }
    variants
}

/// The digit of a single-digit word or token. "ein" and "eine" are left out:
/// they are also the German article. "oh" is left out: it is also a filler
/// word, and a system name can have the letter "O".
fn single_digit(word: &str) -> Option<char> {
    if word.len() == 1 && word.chars().all(|c| c.is_ascii_digit()) {
        return word.chars().next();
    }
    let digit = match word {
        "zero" | "null" => '0',
        "one" | "eins" => '1',
        "two" | "zwei" | "zwo" => '2',
        "three" | "drei" => '3',
        "four" | "vier" => '4',
        "five" | "fünf" | "fuenf" => '5',
        "six" | "sechs" => '6',
        "seven" | "sieben" => '7',
        "eight" | "acht" => '8',
        "nine" | "neun" => '9',
        _ => return None,
    };
    Some(digit)
}

/// The text with each run of two or more single digits joined into one
/// number. A digit word with punctuation after it ends the run. `None` when
/// the text has no such run.
#[cfg(test)]
fn digit_runs(text: &str) -> Option<String> {
    digit_runs_with_positions(text).map(|(text, _)| text)
}

/// As `digit_runs`, with the positions of the joined numbers in the
/// normalised words of the result.
fn digit_runs_with_positions(text: &str) -> Option<(String, Vec<usize>)> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut out: Vec<String> = Vec::new();
    let mut positions: Vec<usize> = Vec::new();
    let mut normalised = 0;
    let mut changed = false;
    let mut i = 0;
    while i < words.len() {
        let mut run = String::new();
        let mut j = i;
        let mut tail = "";
        while j < words.len() {
            let stripped = words[j].trim_end_matches(TRAILING_PUNCT);
            let Some(d) = single_digit(&stripped.to_lowercase()) else {
                break;
            };
            run.push(d);
            j += 1;
            if stripped.len() != words[j - 1].len() {
                tail = &words[j - 1][stripped.len()..];
                break;
            }
        }
        if run.len() >= 2 {
            positions.push(normalised);
            let piece = format!("{run}{tail}");
            normalised += normalize(&piece).len();
            out.push(piece);
            changed = true;
            i = j;
        } else {
            normalised += normalize(words[i]).len();
            out.push(words[i].to_string());
            i += 1;
        }
    }
    changed.then(|| (out.join(" "), positions))
}

/// Writes "fifty-five" as "fifty five". Only a word of two parts that are
/// both number words changes.
fn split_hyphenated(text: &str) -> String {
    let words: Vec<String> = text
        .split_whitespace()
        .map(|word| {
            let stripped = word.trim_end_matches(TRAILING_PUNCT);
            let parts: Vec<&str> = stripped.split('-').collect();
            let is_number_word =
                |p: &str| !matches!(classify(&p.to_lowercase()), None | Some(Kind::And));
            if parts.len() == 2 && parts.iter().all(|p| is_number_word(p)) {
                format!("{} {}{}", parts[0], parts[1], &word[stripped.len()..])
            } else {
                word.to_string()
            }
        })
        .collect();
    words.join(" ")
}

fn tokenize(text: &str) -> Vec<Token<'_>> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut tokens = Vec::with_capacity(words.len());
    for (i, &word) in words.iter().enumerate() {
        let stripped = word.trim_end_matches(TRAILING_PUNCT);
        let punct = stripped.len() != word.len();
        let lower = stripped.to_lowercase();
        let digits = digit_group(stripped);
        let thousands = whole_thousands(stripped, digits.as_deref());
        let kind = if digits.is_some() || stripped.is_empty() {
            None
        } else if lower == "a" {
            let next = words.get(i + 1).map(|w| w.to_lowercase());
            let scale = matches!(next.as_deref(), Some("hundred" | "thousand"));
            (scale && !punct).then_some(Kind::A)
        } else {
            classify(&lower)
        };
        tokens.push(Token {
            word,
            kind,
            digits,
            thousands,
            punct,
        });
    }
    tokens
}

/// For each index, the end of the slice a number that starts there can use:
/// just after the first word with trailing punctuation.
fn segment_ends(tokens: &[Token]) -> Vec<usize> {
    let n = tokens.len();
    let mut ends = vec![n; n];
    for i in (0..n).rev() {
        ends[i] = if tokens[i].punct {
            i + 1
        } else if i + 1 < n {
            ends[i + 1]
        } else {
            n
        };
    }
    ends
}

fn render(
    tokens: &[Token],
    full_kinds: &[Option<Kind>],
    unit_kinds: &[Option<Kind>],
    seg_end: &[usize],
    pass: Pass,
) -> (String, bool) {
    let units_only = pass == Pass::Units;
    let mut out: Vec<String> = Vec::with_capacity(tokens.len());
    let mut changed = false;
    let mut i = 0;
    while i < tokens.len() {
        let token = &tokens[i];
        if let (Pass::Thousands, Some(count)) = (pass, token.thousands) {
            out.push(format!("{count} thousand"));
            changed = true;
            i += 1;
            continue;
        }
        if let Some(digits) = &token.digits {
            out.push(digits.clone());
            changed = true;
            i += 1;
            continue;
        }
        if full_kinds[i].is_none() {
            out.push(token.word.to_string());
            i += 1;
            continue;
        }
        let full = parse_number(&full_kinds[..seg_end[i]], i);
        if let Some((value, next)) = full {
            if value > MAX_NUMBER {
                out.extend(tokens[i..next].iter().map(|t| t.word.to_string()));
                i = next;
                continue;
            }
            if !units_only {
                out.push(value.to_string());
                changed = true;
                i = next;
                continue;
            }
        }
        let units = match unit_kinds[i] {
            Some(Kind::Unit(0)) => Some((0, i + 1)),
            Some(Kind::Compound(value, false)) => Some((value, i + 1)),
            _ => parse_below_100(&unit_kinds[..seg_end[i]], i),
        };
        match units {
            Some((value, next)) => {
                out.push(value.to_string());
                changed = true;
                i = next;
            }
            None => {
                out.push(token.word.to_string());
                i += 1;
            }
        }
    }
    (out.join(" "), changed)
}

fn kind_at(kinds: &[Option<Kind>], i: usize) -> Option<Kind> {
    kinds.get(i).copied().flatten()
}

/// 1 to 99, or a tens word with a unit word. Returns the value and the next index.
fn parse_below_100(kinds: &[Option<Kind>], i: usize) -> Option<(u64, usize)> {
    match kind_at(kinds, i)? {
        Kind::Tens(t) => match kind_at(kinds, i + 1) {
            Some(Kind::Unit(u)) if (1..=9).contains(&u) => Some((t + u, i + 2)),
            _ => Some((t, i + 1)),
        },
        Kind::Unit(n) if n >= 1 => Some((n, i + 1)),
        _ => None,
    }
}

/// 1 to 999, with an optional "hundred" part.
fn parse_below_1000(kinds: &[Option<Kind>], i: usize) -> Option<(u64, usize)> {
    if let (Some(Kind::Unit(u)), Some(Kind::Hundred)) = (kind_at(kinds, i), kind_at(kinds, i + 1)) {
        if (1..=9).contains(&u) {
            let hundreds = u * 100;
            let j = i + 2;
            if kind_at(kinds, j) == Some(Kind::And) {
                if let Some((rest, end)) = parse_below_100(kinds, j + 1) {
                    return Some((hundreds + rest, end));
                }
            }
            if let Some((rest, end)) = parse_below_100(kinds, j) {
                return Some((hundreds + rest, end));
            }
            return Some((hundreds, j));
        }
    }
    parse_below_100(kinds, i)
}

/// The longest valid number that starts at `i`. Returns the value and the next index.
fn parse_number(kinds: &[Option<Kind>], i: usize) -> Option<(u64, usize)> {
    match kind_at(kinds, i)? {
        Kind::Unit(0) => return Some((0, i + 1)),
        Kind::Compound(value, _) => return Some((value, i + 1)),
        _ => {}
    }
    let mut total = 0;
    let mut j = i;
    let mut any = false;
    if let Some((m, end)) = parse_below_1000(kinds, j) {
        if kind_at(kinds, end) == Some(Kind::Million) {
            total += m * 1_000_000;
            j = end + 1;
            any = true;
        }
    }
    if let Some((t, end)) = parse_below_1000(kinds, j) {
        if kind_at(kinds, end) == Some(Kind::Thousand) {
            total += t * 1000;
            j = end + 1;
            any = true;
        }
    }
    if any && kind_at(kinds, j) == Some(Kind::And) {
        if let Some((rest, end)) = parse_below_1000(kinds, j + 1) {
            return Some((total + rest, end));
        }
    }
    if let Some((rest, end)) = parse_below_1000(kinds, j) {
        total += rest;
        j = end;
        any = true;
    }
    any.then_some((total, j))
}

/// A single word from either language, without compounds.
fn simple_word(w: &str) -> Option<Kind> {
    let unit = |n| Some(Kind::Unit(n));
    let tens = |n| Some(Kind::Tens(n));
    match w {
        "zero" | "null" => unit(0),
        "one" | "ein" | "eins" | "eine" => unit(1),
        "two" | "zwei" => unit(2),
        "three" | "drei" => unit(3),
        "four" | "vier" => unit(4),
        "five" | "fünf" => unit(5),
        "six" | "sechs" => unit(6),
        "seven" | "sieben" => unit(7),
        "eight" | "acht" => unit(8),
        "nine" | "neun" => unit(9),
        "ten" | "zehn" => unit(10),
        "eleven" | "elf" => unit(11),
        "twelve" | "zwölf" => unit(12),
        "thirteen" | "dreizehn" => unit(13),
        "fourteen" | "vierzehn" => unit(14),
        "fifteen" | "fünfzehn" => unit(15),
        "sixteen" | "sechzehn" => unit(16),
        "seventeen" | "siebzehn" => unit(17),
        "eighteen" | "achtzehn" => unit(18),
        "nineteen" | "neunzehn" => unit(19),
        "twenty" | "zwanzig" => tens(20),
        "thirty" | "dreißig" | "dreissig" => tens(30),
        "forty" | "vierzig" => tens(40),
        "fifty" | "fünfzig" => tens(50),
        "sixty" | "sechzig" => tens(60),
        "seventy" | "siebzig" => tens(70),
        "eighty" | "achtzig" => tens(80),
        "ninety" | "neunzig" => tens(90),
        _ => None,
    }
}

/// Classify a lowercase word without trailing punctuation.
/// True for a number word of this module, in English or German ("four",
/// "twenty", "hundred", "fünfundfünfzig"). "and" is no number word. `w`
/// must be in lower case.
pub(crate) fn is_number_word(w: &str) -> bool {
    classify(w).is_some_and(|kind| kind != Kind::And)
}

fn classify(w: &str) -> Option<Kind> {
    match w {
        "hundred" | "hundert" => Some(Kind::Hundred),
        "thousand" | "tausend" => Some(Kind::Thousand),
        "million" => Some(Kind::Million),
        "and" => Some(Kind::And),
        _ => simple_word(w).or_else(|| german_compound(w)),
    }
}

/// A German number written as one word, such as `fünfundfünfzig`,
/// `einhundertzwanzig` or `zweitausend`.
fn german_compound(w: &str) -> Option<Kind> {
    if let Some(p) = w.find("tausend") {
        let left = &w[..p];
        let right = &w[p + "tausend".len()..];
        let thousands = if left.is_empty() {
            1
        } else {
            german_below_1000(left)?
        };
        let rest = if right.is_empty() {
            0
        } else {
            german_below_1000(right)?
        };
        return Some(Kind::Compound(thousands * 1000 + rest, true));
    }
    let scale = w.contains("hundert");
    german_below_1000(w).map(|value| Kind::Compound(value, scale))
}

fn german_below_1000(w: &str) -> Option<u64> {
    if let Some(p) = w.find("hundert") {
        let left = &w[..p];
        let right = &w[p + "hundert".len()..];
        let hundreds = if left.is_empty() {
            1
        } else {
            match simple_word(left) {
                Some(Kind::Unit(u)) if (1..=9).contains(&u) => u,
                _ => return None,
            }
        };
        let rest = if right.is_empty() {
            0
        } else {
            german_below_100(right)?
        };
        return Some(hundreds * 100 + rest);
    }
    german_below_100(w)
}

fn german_below_100(w: &str) -> Option<u64> {
    match simple_word(w) {
        Some(Kind::Unit(n)) if n >= 1 => return Some(n),
        Some(Kind::Tens(t)) => return Some(t),
        _ => {}
    }
    for (p, _) in w.match_indices("und") {
        if let (Some(Kind::Unit(u)), Some(Kind::Tens(t))) =
            (simple_word(&w[..p]), simple_word(&w[p + "und".len()..]))
        {
            if (1..=9).contains(&u) {
                return Some(u + t);
            }
        }
    }
    None
}

/// A group of digits with a thousands separator, such as `5,000` or `5.000`.
/// Returns the digits without the separators. A group with a leading zero,
/// such as `0.500`, is not a thousands group.
fn digit_group(word: &str) -> Option<String> {
    let sep = word.chars().find(|c| matches!(c, ',' | '.'))?;
    let mut parts = word.split(sep);
    let first = parts.next()?;
    if first.is_empty()
        || first.len() > 3
        || first.starts_with('0')
        || !first.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let mut digits = first.to_string();
    let mut groups = 0;
    for part in parts {
        if part.len() != 3 || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        digits.push_str(part);
        groups += 1;
    }
    (groups > 0).then_some(digits)
}

/// The count of thousands when the digit group `group`, or else the plain
/// number `word`, is 1000 or more and a whole number of thousands. A plain
/// number with a leading zero gives `None`.
fn whole_thousands(word: &str, group: Option<&str>) -> Option<u64> {
    let digits = match group {
        Some(digits) => digits,
        None if !word.is_empty()
            && !word.starts_with('0')
            && word.bytes().all(|b| b.is_ascii_digit()) =>
        {
            word
        }
        None => return None,
    };
    let value: u64 = digits.parse().ok()?;
    (value >= 1000 && value.is_multiple_of(1000)).then_some(value / 1000)
}

#[cfg(test)]
mod tests {
    use super::{digit_runs, number_variants};

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn test_digit_runs_become_one_number() {
        let variants = number_variants("sail out to ktl three zero four eight one");
        assert!(
            variants.contains(&"sail out to ktl 30481".to_string()),
            "{variants:?}"
        );
        let variants = number_variants("Sail out to ktl one seven five one nine.");
        assert!(
            variants.contains(&"Sail out to ktl 17519.".to_string()),
            "{variants:?}"
        );
        assert_eq!(digit_runs("ktl 1 7 5 1 9").as_deref(), Some("ktl 17519"));
        assert_eq!(
            digit_runs("kurs auf ktl drei null vier acht eins").as_deref(),
            Some("kurs auf ktl 30481")
        );
        // A run needs two digits. Punctuation ends a run.
        assert_eq!(digit_runs("go to one"), None);
        assert_eq!(digit_runs("one, two").as_deref(), None);
        assert_eq!(digit_runs("one two, three").as_deref(), Some("12, three"));
        // No pair forms, no article, no "oh".
        assert_eq!(digit_runs("ten eight twenty eight"), None);
        assert_eq!(digit_runs("ein zwei"), None);
        assert_eq!(digit_runs("oh nine"), None);
    }

    /// The earlier readings stay first: "three four" is still "3 4".
    #[test]
    fn test_digit_runs_come_after_the_other_readings() {
        assert_eq!(number_variants("three four"), v(&["3 4", "34"]));
        assert_eq!(number_variants("fifty five"), v(&["55"]));
    }

    #[test]
    fn test_english_tens_and_units() {
        assert_eq!(
            number_variants("bell volume fifty five"),
            v(&["bell volume 55"])
        );
    }

    #[test]
    fn test_english_hundreds() {
        assert_eq!(
            number_variants("net reminder one hundred twenty"),
            v(&["net reminder 120", "net reminder 1 hundred 20"])
        );
        assert_eq!(number_variants("one hundred and twenty")[0], "120");
        assert_eq!(number_variants("a hundred")[0], "100");
    }

    #[test]
    fn test_thousand_keeps_a_units_variant() {
        assert_eq!(
            number_variants("go to twelve thousand sea miles"),
            v(&["go to 12000 sea miles", "go to 12 thousand sea miles"])
        );
    }

    #[test]
    fn test_separate_digits_come_before_the_digit_run() {
        // The separate reading comes first. The digit run is a later reading.
        assert_eq!(
            number_variants("pick three four"),
            v(&["pick 3 4", "pick 34"])
        );
        assert_eq!(number_variants("twenty three"), v(&["23"]));
        assert_eq!(number_variants("three twenty"), v(&["3 20"]));
    }

    #[test]
    fn test_german() {
        assert_eq!(
            number_variants("lautstärke fünfundfünfzig"),
            v(&["lautstärke 55"])
        );
        assert_eq!(number_variants("einhundertzwanzig")[0], "120");
        assert_eq!(number_variants("zwölf"), v(&["12"]));
        assert_eq!(number_variants("zweitausend")[0], "2000");
        assert_eq!(number_variants("ein hundert zwanzig")[0], "120");
    }

    #[test]
    fn test_digit_groups() {
        assert_eq!(
            number_variants("buoy range 5,000"),
            v(&["buoy range 5000", "buoy range 5 thousand"])
        );
        assert_eq!(
            number_variants("buoy range 5.000"),
            v(&["buoy range 5000", "buoy range 5 thousand"])
        );
        assert_eq!(number_variants("buoy range 5,500"), v(&["buoy range 5500"]));
        assert_eq!(number_variants("version 1.5"), Vec::<String>::new());
    }

    #[test]
    fn test_hyphenated_number_words() {
        assert_eq!(
            number_variants("bell volume fifty-five"),
            v(&["bell volume 55"])
        );
        assert_eq!(number_variants("twenty-five"), v(&["25"]));
        assert_eq!(number_variants("Twenty-Five."), v(&["25"]));
        assert_eq!(number_variants("well-known x-ray"), Vec::<String>::new());
        assert_eq!(number_variants("twenty-something"), Vec::<String>::new());
        assert_eq!(number_variants("one-two-three"), Vec::<String>::new());
    }

    #[test]
    fn test_digit_group_with_sentence_punctuation() {
        assert_eq!(
            number_variants("buoy range 5,000."),
            v(&["buoy range 5000", "buoy range 5 thousand"])
        );
        assert_eq!(
            number_variants("go to 12.000 sea miles."),
            v(&["go to 12000 sea miles.", "go to 12 thousand sea miles."])
        );
        assert_eq!(
            number_variants("go to 12.000."),
            v(&["go to 12000", "go to 12 thousand"])
        );
    }

    #[test]
    fn test_digit_group_with_leading_zero_stays() {
        assert_eq!(number_variants("gain 0.500"), Vec::<String>::new());
        assert_eq!(number_variants("gain 0,500"), Vec::<String>::new());
        assert_eq!(number_variants("gain 0.500."), Vec::<String>::new());
    }

    #[test]
    fn test_whole_thousands_get_a_thousand_variant() {
        assert_eq!(
            number_variants("go to 12000 sea miles"),
            v(&["go to 12 thousand sea miles"])
        );
        assert_eq!(
            number_variants("go to 12,000 sea miles"),
            v(&["go to 12000 sea miles", "go to 12 thousand sea miles"])
        );
        assert_eq!(number_variants("1000"), v(&["1 thousand"]));
        // Not a whole thousand, below 1000, or a leading zero: no variant.
        assert_eq!(number_variants("12500"), Vec::<String>::new());
        assert_eq!(number_variants("999 0 500"), Vec::<String>::new());
        assert_eq!(number_variants("012000"), Vec::<String>::new());
    }

    #[test]
    fn test_zero() {
        assert_eq!(number_variants("zero"), v(&["0"]));
        assert_eq!(number_variants("null"), v(&["0"]));
    }

    #[test]
    fn test_words_that_contain_number_words_stay() {
        assert_eq!(
            number_variants("someone toned the stone"),
            Vec::<String>::new()
        );
        assert_eq!(number_variants("einfach"), Vec::<String>::new());
    }

    #[test]
    fn test_no_numbers() {
        assert_eq!(number_variants("help chart log"), Vec::<String>::new());
        assert_eq!(number_variants(""), Vec::<String>::new());
    }

    #[test]
    fn test_large_number_limit() {
        // Above 9,999,999: no panic, and the words stay unchanged.
        assert_eq!(number_variants("ten million"), Vec::<String>::new());
        assert_eq!(
            number_variants("ninety nine million nine hundred thousand"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn test_punctuation_on_number_words_is_dropped() {
        assert_eq!(
            number_variants("set volume to fifty, please"),
            v(&["set volume to 50 please"])
        );
        assert_eq!(number_variants("five."), v(&["5"]));
    }
}
