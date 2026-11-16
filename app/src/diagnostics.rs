//! The utterance log and the audio capture, for tuning with real speech.
//!
//! Both run on the worker thread. A write error prints one warning and
//! switches the part off. It never stops the session. A clip that the
//! retention cannot delete gives one warning and the capture continues.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use edcopilot_voicebridge_stt::{wav, AudioClip};

use crate::session::Outcome;

/// The audio capture keeps at most this many clips. It deletes the oldest.
pub const MAX_SAVED_CLIPS: usize = 500;

/// The start of the name of each saved clip.
const CLIP_PREFIX: &str = "utt-";
const CLIP_EXTENSION: &str = ".wav";

/// UTC time in ISO-8601 with milliseconds: `2026-11-14T09:05:03.042Z`.
pub fn iso8601_utc(time: SystemTime) -> String {
    let since = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    let (date, clock) = utc_parts(since);
    format!("{date}T{clock}Z")
}

/// (`2026-11-14`, `09:05:03.042`) for a time after 1970.
fn utc_parts(since: Duration) -> (String, String) {
    let secs = since.as_secs();
    let days = (secs / 86_400) as i64;
    let rest = secs % 86_400;
    let (year, month, day) = civil_from_days(days);
    (
        format!("{year:04}-{month:02}-{day:02}"),
        format!(
            "{:02}:{:02}:{:02}.{:03}",
            rest / 3600,
            rest % 3600 / 60,
            rest % 60,
            since.subsec_millis()
        ),
    )
}

/// Year, month, day of a day count from 1970-01-01 (H. Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// The file name of a saved clip: it sorts by time and the counter makes it unique.
pub fn clip_file_name(time: SystemTime, counter: u64) -> String {
    let since = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    let (date, clock) = utc_parts(since);
    let date = date.replace('-', "");
    let clock = clock.replace([':', '.'], "");
    format!("{CLIP_PREFIX}{date}T{clock}Z-{counter:06}{CLIP_EXTENSION}")
}

/// True for a name that `clip_file_name` makes:
/// `utt-YYYYMMDDTHHMMSSmmmZ-NNNNNN.wav` (the counter has 6 digits or more).
pub fn is_clip_name(name: &str) -> bool {
    let Some(rest) = name
        .strip_prefix(CLIP_PREFIX)
        .and_then(|r| r.strip_suffix(CLIP_EXTENSION))
    else {
        return false;
    };
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let Some((stamp, counter)) = rest.split_once("Z-") else {
        return false;
    };
    let Some((date, clock)) = stamp.split_once('T') else {
        return false;
    };
    date.len() == 8
        && digits(date)
        && clock.len() == 9
        && digits(clock)
        && counter.len() >= 6
        && digits(counter)
}

/// The clips to delete so that `keep` clips stay: the oldest names first.
/// Only clip names (see `is_clip_name`) are in the result, and never
/// `protect` (the clip just written).
pub fn clips_to_delete(names: &[String], keep: usize, protect: &str) -> Vec<String> {
    let mut clips: Vec<&String> = names.iter().filter(|n| is_clip_name(n)).collect();
    clips.sort();
    let extra = clips.len().saturating_sub(keep);
    clips
        .into_iter()
        .take(extra)
        .filter(|n| n.as_str() != protect)
        .cloned()
        .collect()
}

/// The level of silence, and the lowest level that `levels` gives.
pub const LEVEL_FLOOR_DBFS: f32 = -120.0;

/// The peak and RMS level of a clip in dBFS.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Levels {
    pub peak_dbfs: f32,
    pub rms_dbfs: f32,
}

/// A peak at or above this level is clipping.
pub const CLIPPING_DBFS: f32 = -0.1;
/// A peak below this level is very quiet. The local engine gave the same
/// text for one clip at 0, -20, -30 and -40 dBFS, so -40 is not a problem.
pub const VERY_QUIET_DBFS: f32 = -50.0;

impl Levels {
    pub fn clipping(&self) -> bool {
        self.peak_dbfs >= CLIPPING_DBFS
    }

    pub fn very_quiet(&self) -> bool {
        self.peak_dbfs < VERY_QUIET_DBFS
    }
}

/// The peak and RMS level of the samples, full scale 1.0. Silence, an empty
/// clip and very low levels give `LEVEL_FLOOR_DBFS`. A sample that is not a
/// finite number counts as 0. The result is never NaN or infinite.
pub fn levels(samples: &[f32]) -> Levels {
    let mut peak = 0.0_f64;
    let mut squares = 0.0_f64;
    for &s in samples {
        let s = if s.is_finite() {
            f64::from(s.abs())
        } else {
            0.0
        };
        peak = peak.max(s);
        squares += s * s;
    }
    let rms = if samples.is_empty() {
        0.0
    } else {
        (squares / samples.len() as f64).sqrt()
    };
    Levels {
        peak_dbfs: dbfs(peak),
        rms_dbfs: dbfs(rms),
    }
}

/// `20 log10(amplitude)`, not below the floor.
fn dbfs(amplitude: f64) -> f32 {
    if amplitude > 0.0 {
        ((20.0 * amplitude.log10()) as f32).max(LEVEL_FLOOR_DBFS)
    } else {
        LEVEL_FLOOR_DBFS
    }
}

/// A text field without tabs and line breaks.
fn field(text: &str) -> String {
    text.replace(char::is_control, " ")
}

/// The kind, heard text, token and confidence of an outcome.
fn outcome_fields(outcome: &Outcome) -> (&'static str, &str, &str, Option<f32>) {
    match outcome {
        Outcome::Sent {
            text,
            token,
            confidence,
            ..
        } => ("sent", text, token, Some(*confidence)),
        Outcome::DryRun {
            text,
            token,
            confidence,
            ..
        } => ("dry_run", text, token, Some(*confidence)),
        Outcome::NoMatch { text, .. } => ("no_match", text, "", None),
        Outcome::NoWakeWord { text } => ("no_wake_word", text, "", None),
        Outcome::Rejected {
            text,
            token,
            confidence,
            ..
        } => ("rejected", text, token, Some(*confidence)),
        Outcome::NotConsumed { text, token, .. } => ("not_consumed", text, token, None),
        Outcome::EngineError { .. } => ("engine_error", "", "", None),
        Outcome::SendError { text, token, .. } => ("send_error", text, token, None),
    }
}

/// The marks of an utterance that the outcome and the levels do not show.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Marks {
    /// The audio stream lost samples during the clip.
    pub glitch: bool,
    /// The outcome came from the second pass of the engine (the hotwords).
    pub hotwords: bool,
}

/// The marks of an utterance, separated by spaces: `synonym`, `sound_alike`,
/// `filler`, `hotwords`, `clipping`, `very_quiet`, `glitch`, then for a
/// blocked result `blocked_` and the name of the cause (`blocked_tie`,
/// `blocked_required_words`, `blocked_no_token`, `blocked_forms`,
/// `blocked_dropped_word`).
pub fn flags(outcome: Option<&Outcome>, levels: Levels, marks: Marks) -> String {
    let sound_alike = matches!(
        outcome,
        Some(
            Outcome::Sent {
                sound_alike: true,
                ..
            } | Outcome::DryRun {
                sound_alike: true,
                ..
            } | Outcome::Rejected {
                sound_alike: true,
                ..
            }
        )
    );
    let synonym = matches!(
        outcome,
        Some(
            Outcome::Sent { synonym: true, .. }
                | Outcome::DryRun { synonym: true, .. }
                | Outcome::Rejected { synonym: true, .. }
        )
    );
    let filler = matches!(
        outcome,
        Some(
            Outcome::Sent { filler: true, .. }
                | Outcome::DryRun { filler: true, .. }
                | Outcome::Rejected { filler: true, .. }
        )
    );
    let blocked = match outcome {
        Some(Outcome::NoMatch {
            blocked: Some(reason),
            ..
        }) => Some(format!("blocked_{}", reason.name())),
        _ => None,
    };
    [
        (synonym, "synonym"),
        (sound_alike, "sound_alike"),
        (filler, "filler"),
        (marks.hotwords, "hotwords"),
        (levels.clipping(), "clipping"),
        (levels.very_quiet(), "very_quiet"),
        (marks.glitch, "glitch"),
    ]
    .iter()
    .filter(|(on, _)| *on)
    .map(|(_, name)| name.to_string())
    .chain(blocked)
    .collect::<Vec<_>>()
    .join(" ")
}

/// One tab-separated record without a line break: time, kind, heard text,
/// token, confidence, transcribe ms, total ms, audio file name, peak dBFS,
/// RMS dBFS, flags. New fields go at the end, so that old lines stay readable.
pub fn format_record(
    timestamp: &str,
    outcome: Option<&Outcome>,
    transcribe: Duration,
    total: Duration,
    audio: &str,
    levels: Levels,
    marks: Marks,
) -> String {
    let (kind, text, token, confidence) = match outcome {
        Some(outcome) => outcome_fields(outcome),
        None => ("dropped", "", "", None),
    };
    let confidence = confidence.map(|c| format!("{c:.2}")).unwrap_or_default();
    [
        field(timestamp),
        kind.to_string(),
        field(text),
        field(token),
        confidence,
        transcribe.as_millis().to_string(),
        total.as_millis().to_string(),
        field(audio),
        format!("{:.1}", levels.peak_dbfs),
        format!("{:.1}", levels.rms_dbfs),
        flags(outcome, levels, marks),
    ]
    .join("\t")
}

/// The utterance log and the audio capture of one session.
pub struct Diagnostics {
    log: Option<File>,
    audio_dir: Option<PathBuf>,
    counter: u64,
    /// A retention problem was reported. It is reported once per session.
    prune_warned: bool,
    /// Gets each warning line. The default prints it on stderr.
    warn: Box<dyn Fn(String) + Send>,
}

/// The default target of the warning lines.
fn print_warning(line: String) {
    eprintln!("{line}");
}

impl Diagnostics {
    /// A part with an empty path is off. A log that does not open prints a
    /// warning and stays off.
    pub fn new(log_file: &Path, audio_dir: &Path) -> Diagnostics {
        Diagnostics::with_warnings(log_file, audio_dir, print_warning)
    }

    /// As `new`, but each warning line goes to `warn`, not to stderr. The
    /// line is the complete text that the console prints.
    pub fn with_warnings(
        log_file: &Path,
        audio_dir: &Path,
        warn: impl Fn(String) + Send + 'static,
    ) -> Diagnostics {
        let log = (!log_file.as_os_str().is_empty())
            .then(|| {
                OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(log_file)
                    .map_err(|e| {
                        warn(format!(
                            "warning: cannot open the utterance log {}, no log: {e}",
                            log_file.display()
                        ))
                    })
                    .ok()
            })
            .flatten();
        let audio_dir = (!audio_dir.as_os_str().is_empty()).then(|| audio_dir.to_path_buf());
        Diagnostics {
            log,
            audio_dir,
            counter: 0,
            prune_warned: false,
            warn: Box::new(warn),
        }
    }

    /// Both parts off.
    pub fn off() -> Diagnostics {
        Diagnostics {
            log: None,
            audio_dir: None,
            counter: 0,
            prune_warned: false,
            warn: Box::new(print_warning),
        }
    }

    pub fn log_enabled(&self) -> bool {
        self.log.is_some()
    }

    pub fn audio_enabled(&self) -> bool {
        self.audio_dir.is_some()
    }

    /// Saves the clip as a 32-bit float WAV file and returns its name. The
    /// file holds the samples that the engine got, bit for bit, so that a
    /// replay gives the engine the same input. `None` when the
    /// capture is off or the write failed. Deletes the oldest clips beyond
    /// `MAX_SAVED_CLIPS`, never the clip just written. A failed delete gives
    /// one warning per session and the capture continues.
    pub fn save_clip(&mut self, clip: &AudioClip, time: SystemTime) -> Option<String> {
        let dir = self.audio_dir.clone()?;
        self.counter += 1;
        let name = clip_file_name(time, self.counter);
        let written = fs::create_dir_all(&dir)
            .and_then(|()| fs::write(dir.join(&name), wav::encode_f32(clip)));
        if let Err(e) = written {
            (self.warn)(format!(
                "warning: cannot save audio in {}, audio capture is off: {e}",
                dir.display()
            ));
            self.audio_dir = None;
            return None;
        }
        if let Err(e) = prune(&dir, MAX_SAVED_CLIPS, &name) {
            if !self.prune_warned {
                (self.warn)(format!(
                    "warning: cannot delete old audio files in {}: {e}",
                    dir.display()
                ));
                self.prune_warned = true;
            }
        }
        Some(name)
    }

    /// Appends one record. `outcome` is `None` for a dropped utterance.
    /// `times` is the transcription time and the total time.
    pub fn log(
        &mut self,
        time: SystemTime,
        outcome: Option<&Outcome>,
        (transcribe, total): (Duration, Duration),
        audio: Option<&str>,
        levels: Levels,
        marks: Marks,
    ) {
        let Some(file) = self.log.as_mut() else {
            return;
        };
        let line = format_record(
            &iso8601_utc(time),
            outcome,
            transcribe,
            total,
            audio.unwrap_or(""),
            levels,
            marks,
        );
        if let Err(e) = writeln!(file, "{line}").and_then(|()| file.flush()) {
            (self.warn)(format!(
                "warning: cannot write the utterance log, no log: {e}"
            ));
            self.log = None;
        }
    }
}

/// Deletes the oldest clips in `dir` so that `keep` clips stay, never
/// `protect`. It tries each clip and gives the first error.
fn prune(dir: &Path, keep: usize, protect: &str) -> std::io::Result<()> {
    let names: Vec<String> = fs::read_dir(dir)?
        .filter_map(Result::ok)
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    let mut first_error = None;
    for name in clips_to_delete(&names, keep, protect) {
        if let Err(e) = fs::remove_file(dir.join(name)) {
            first_error.get_or_insert(e);
        }
    }
    first_error.map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use edcopilot_voicebridge_core::BlockReason;

    fn at(secs: u64, millis: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs) + Duration::from_millis(millis)
    }

    #[test]
    fn test_iso8601_utc() {
        assert_eq!(iso8601_utc(at(0, 0)), "1970-01-01T00:00:00.000Z");
        // 2026-11-14 09:05:03.042 UTC.
        assert_eq!(
            iso8601_utc(at(1_794_647_103, 42)),
            "2026-11-14T09:05:03.042Z"
        );
        // A leap day.
        assert_eq!(iso8601_utc(at(951_782_400, 0)), "2000-02-29T00:00:00.000Z");
    }

    #[test]
    fn test_clip_names_sort_by_time_and_are_unique() {
        let a = clip_file_name(at(1_794_647_103, 42), 1);
        assert_eq!(a, "utt-20261114T090503042Z-000001.wav");
        let b = clip_file_name(at(1_794_647_103, 42), 2);
        let c = clip_file_name(at(1_794_647_104, 0), 3);
        assert_ne!(a, b);
        assert!(a < b && b < c);
    }

    #[test]
    fn test_clips_to_delete_keeps_the_newest() {
        let names: Vec<String> = [
            "utt-20261114T090503042Z-000003.wav",
            "notes.txt",
            "utt-20261114T090503042Z-000001.wav",
            "utt-20261114T090503042Z-000002.wav",
            "other.wav",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(
            clips_to_delete(&names, 1, ""),
            vec![
                "utt-20261114T090503042Z-000001.wav",
                "utt-20261114T090503042Z-000002.wav"
            ]
        );
        assert!(clips_to_delete(&names, 3, "").is_empty());
        assert_eq!(clips_to_delete(&names, 0, "").len(), 3);
    }

    fn assert_near(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 0.1,
            "{actual} is not {expected}"
        );
    }

    fn sine(amplitude: f32) -> Vec<f32> {
        // 1 kHz at 16 kHz: 16 samples for each period, 1000 periods.
        (0..16_000)
            .map(|i| amplitude * (2.0 * std::f32::consts::PI * i as f32 / 16.0).sin())
            .collect()
    }

    #[test]
    fn test_levels_of_full_scale_signals() {
        let l = levels(&sine(1.0));
        assert_near(l.peak_dbfs, 0.0);
        assert_near(l.rms_dbfs, -3.01);
        let l = levels(&[1.0; 1_000]);
        assert_near(l.peak_dbfs, 0.0);
        assert_near(l.rms_dbfs, 0.0);
        let l = levels(&[-1.0; 1_000]);
        assert_near(l.peak_dbfs, 0.0);
    }

    #[test]
    fn test_levels_of_known_amplitudes() {
        let l = levels(&sine(0.1));
        assert_near(l.peak_dbfs, -20.0);
        assert_near(l.rms_dbfs, -23.01);
        let l = levels(&[0.5; 1_000]);
        assert_near(l.peak_dbfs, -6.02);
        assert_near(l.rms_dbfs, -6.02);
    }

    #[test]
    fn test_levels_of_silence_are_the_floor() {
        let floor = Levels {
            peak_dbfs: LEVEL_FLOOR_DBFS,
            rms_dbfs: LEVEL_FLOOR_DBFS,
        };
        assert_eq!(levels(&[]), floor);
        assert_eq!(levels(&[0.0; 16_000]), floor);
        // Very small values and values that are not numbers stay finite.
        let l = levels(&[1e-30, f32::NAN, f32::INFINITY, 0.0]);
        assert!(l.peak_dbfs.is_finite() && l.rms_dbfs.is_finite(), "{l:?}");
        assert_eq!(l, floor);
    }

    #[test]
    fn test_record_of_a_sent_outcome() {
        let outcome = Outcome::Sent {
            text: "cheapest\tby\nbid cork".into(),
            token: "BuyTop:Cork".into(),
            confidence: 0.95,
            sound_alike: true,
            synonym: false,
            filler: false,
        };
        let line = format_record(
            "2026-11-14T09:05:03.042Z",
            Some(&outcome),
            Duration::from_millis(61),
            Duration::from_millis(75),
            "utt-1.wav",
            Levels {
                peak_dbfs: -6.04,
                rms_dbfs: -24.36,
            },
            Marks {
                glitch: true,
                hotwords: true,
            },
        );
        assert_eq!(
            line,
            "2026-11-14T09:05:03.042Z\tsent\tcheapest by bid cork\tBuyTop:Cork\t0.95\t61\t75\tutt-1.wav\t-6.0\t-24.4\tsound_alike hotwords glitch"
        );
    }

    #[test]
    fn test_record_kinds_and_empty_fields() {
        let ms = Duration::from_millis;
        let cases = [
            (
                Some(Outcome::NoMatch {
                    text: "mmm".into(),
                    blocked: None,
                }),
                "T\tno_match\tmmm\t\t\t1\t2\t\t-120.0\t-120.0\tvery_quiet",
            ),
            (
                Some(Outcome::EngineError {
                    message: "boom".into(),
                }),
                "T\tengine_error\t\t\t\t1\t2\t\t-120.0\t-120.0\tvery_quiet",
            ),
            (None, "T\tdropped\t\t\t\t1\t2\t\t-120.0\t-120.0\tvery_quiet"),
        ];
        for (outcome, expected) in cases {
            assert_eq!(
                format_record(
                    "T",
                    outcome.as_ref(),
                    ms(1),
                    ms(2),
                    "",
                    levels(&[]),
                    Marks::default()
                ),
                expected
            );
        }
        let kinds = [
            (
                Outcome::DryRun {
                    text: "a".into(),
                    token: "T".into(),
                    confidence: 1.0,
                    sound_alike: false,
                    synonym: false,
                    filler: false,
                },
                "dry_run",
            ),
            (Outcome::NoWakeWord { text: "a".into() }, "no_wake_word"),
            (
                Outcome::Rejected {
                    text: "a".into(),
                    token: "T".into(),
                    confidence: 0.8,
                    sound_alike: false,
                    synonym: false,
                    filler: false,
                    reason: "r".into(),
                },
                "rejected",
            ),
            (
                Outcome::NotConsumed {
                    text: "a".into(),
                    token: "T".into(),
                    withdrawn: true,
                },
                "not_consumed",
            ),
            (
                Outcome::SendError {
                    text: "a".into(),
                    token: "T".into(),
                    message: "m".into(),
                },
                "send_error",
            ),
        ];
        for (outcome, kind) in kinds {
            let line = format_record(
                "T",
                Some(&outcome),
                ms(0),
                ms(0),
                "",
                levels(&[]),
                Marks::default(),
            );
            assert_eq!(line.split('\t').nth(1), Some(kind), "{line}");
            assert_eq!(line.split('\t').count(), 11, "{line}");
        }
    }

    #[test]
    fn test_log_appends_and_audio_is_saved() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("utterances.tsv");
        let audio = dir.path().join("clips");
        std::fs::write(&log, "old line\n").unwrap();
        let mut d = Diagnostics::new(&log, &audio);
        let clip = AudioClip {
            samples: vec![0.25; 1600],
        };
        let name = d.save_clip(&clip, at(1_794_647_103, 42)).unwrap();
        assert!(audio.join(&name).is_file());
        let back = wav::read_file(&audio.join(&name)).unwrap();
        assert_eq!(back.samples.len(), 1600);
        let outcome = Outcome::NoMatch {
            text: "x".into(),
            blocked: None,
        };
        let ms = Duration::from_millis;
        let quiet = levels(&clip.samples);
        d.log(
            at(0, 0),
            Some(&outcome),
            (ms(1), ms(2)),
            Some(&name),
            quiet,
            Marks::default(),
        );
        d.log(
            at(0, 0),
            None,
            (ms(0), ms(0)),
            None,
            quiet,
            Marks::default(),
        );
        let text = std::fs::read_to_string(&log).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3, "{text}");
        assert_eq!(lines[0], "old line");
        let fields: Vec<&str> = lines[1].split('\t').collect();
        assert_eq!(fields[7], name, "{text}");
        assert_eq!(&fields[8..], ["-12.0", "-12.0", ""], "{text}");
        assert!(lines[2].contains("\tdropped\t"), "{text}");
    }

    /// Replay must give the engine the samples of the live run, bit for bit.
    #[test]
    fn test_saved_clip_loads_bit_equal() {
        let dir = tempfile::tempdir().unwrap();
        let mut d = Diagnostics::new(Path::new(""), dir.path());
        let samples: Vec<f32> = (0..4000)
            .map(|i| 0.2 * (i as f32 * 0.05).sin() + 3e-6 * (i % 7) as f32)
            .chain([1e-40, -0.0, 1.0 / 3.0, 1.2])
            .collect();
        let clip = AudioClip { samples };
        let name = d.save_clip(&clip, at(1_794_647_103, 42)).unwrap();
        let back = wav::read_file(&dir.path().join(name)).unwrap();
        let bits = |s: &[f32]| s.iter().map(|v| v.to_bits()).collect::<Vec<u32>>();
        assert_eq!(bits(&back.samples), bits(&clip.samples));
    }

    #[test]
    fn test_empty_paths_switch_both_parts_off() {
        let mut d = Diagnostics::new(Path::new(""), Path::new(""));
        assert!(!d.log_enabled());
        assert!(!d.audio_enabled());
        let clip = AudioClip { samples: vec![0.0] };
        assert_eq!(d.save_clip(&clip, at(0, 0)), None);
    }

    #[test]
    fn test_write_errors_switch_the_parts_off() {
        let dir = tempfile::tempdir().unwrap();
        // A log path in a directory that does not exist.
        let d = Diagnostics::new(&dir.path().join("no/such/log.tsv"), Path::new(""));
        assert!(!d.log_enabled());
        // An audio directory below a file cannot be made.
        let file = dir.path().join("a-file");
        std::fs::write(&file, "x").unwrap();
        let mut d = Diagnostics::new(Path::new(""), &file.join("clips"));
        let clip = AudioClip { samples: vec![0.0] };
        assert_eq!(d.save_clip(&clip, at(0, 0)), None);
        assert!(!d.audio_enabled());
    }

    #[test]
    fn test_capture_keeps_at_most_the_newest_clips() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..3 {
            std::fs::write(dir.path().join(clip_file_name(at(i, 0), i)), "x").unwrap();
        }
        std::fs::write(dir.path().join("keep.txt"), "x").unwrap();
        prune(dir.path(), 2, "").unwrap();
        let mut names: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(names.len(), 3, "{names:?}");
        assert!(names.contains(&"keep.txt".to_string()));
        assert!(!names.contains(&clip_file_name(at(0, 0), 0)));
    }

    #[test]
    fn test_only_generated_clip_names_are_clip_names() {
        assert!(is_clip_name("utt-20261114T090503042Z-000001.wav"));
        assert!(is_clip_name("utt-20261114T090503042Z-1234567.wav"));
        for name in [
            "utt-notes.wav",
            "utt-.wav",
            "utt-20261114T090503042Z-1.wav",
            "utt-2026111T090503042Z-000001.wav",
            "utt-20261114T090503042Z-00000a.wav",
            "utt-20261114T090503042Z-000001.WAV",
            "utt-20261114-090503042Z-000001.wav",
            "my-utt-20261114T090503042Z-000001.wav",
        ] {
            assert!(!is_clip_name(name), "{name}");
        }
        let names = vec!["utt-notes.wav".to_string(), "utt-1.wav".to_string()];
        assert!(clips_to_delete(&names, 0, "").is_empty());
    }

    #[test]
    fn test_the_clip_just_written_is_never_deleted() {
        let newest = clip_file_name(at(1, 0), 1);
        let older = clip_file_name(at(2, 0), 2);
        // The clock went back: the clip just written sorts first.
        let names = vec![newest.clone(), older.clone()];
        assert_eq!(clips_to_delete(&names, 1, &newest), Vec::<String>::new());
        assert_eq!(clips_to_delete(&names, 0, &newest), vec![older]);
    }

    /// A clip name that cannot be deleted (here a directory) gives a
    /// warning. The capture stays on and the log line keeps the name.
    #[test]
    fn test_failed_delete_keeps_the_capture_and_the_name() {
        let dir = tempfile::tempdir().unwrap();
        let blocked = dir.path().join(clip_file_name(at(0, 0), 0));
        std::fs::create_dir(&blocked).unwrap();
        let mut d = Diagnostics::new(Path::new(""), dir.path());
        let clip = AudioClip {
            samples: vec![0.0; 16],
        };
        for i in 0..(MAX_SAVED_CLIPS as u64 + 2) {
            let name = d.save_clip(&clip, at(100 + i, 0));
            assert!(name.is_some(), "clip {i}");
            assert!(dir.path().join(name.unwrap()).is_file());
        }
        assert!(d.audio_enabled());
        assert!(d.prune_warned);
        assert!(blocked.is_dir());
        let files = std::fs::read_dir(dir.path()).unwrap().count();
        assert_eq!(files, MAX_SAVED_CLIPS + 1);
    }

    #[test]
    fn test_flags_name_the_cause_of_a_block() {
        let normal = Levels {
            peak_dbfs: -10.0,
            rms_dbfs: -20.0,
        };
        for (reason, flag) in [
            (BlockReason::Tie, "blocked_tie"),
            (BlockReason::RequiredWords, "blocked_required_words"),
            (BlockReason::NoToken, "blocked_no_token"),
            (BlockReason::Forms, "blocked_forms"),
            (BlockReason::DroppedWord, "blocked_dropped_word"),
        ] {
            let outcome = Outcome::NoMatch {
                text: "lock lantern".into(),
                blocked: Some(reason),
            };
            assert_eq!(flags(Some(&outcome), normal, Marks::default()), flag);
            let marks = Marks {
                glitch: true,
                hotwords: false,
            };
            assert_eq!(
                flags(Some(&outcome), normal, marks),
                format!("glitch {flag}")
            );
        }
        let plain = Outcome::NoMatch {
            text: "mmm".into(),
            blocked: None,
        };
        assert_eq!(flags(Some(&plain), normal, Marks::default()), "");
    }

    #[test]
    fn test_flags_of_an_utterance() {
        let loud = Levels {
            peak_dbfs: 0.0,
            rms_dbfs: -10.0,
        };
        let normal = Levels {
            peak_dbfs: -10.0,
            rms_dbfs: -20.0,
        };
        let rejected = Outcome::Rejected {
            text: "bye".into(),
            token: "Buy".into(),
            confidence: 0.95,
            sound_alike: true,
            synonym: false,
            filler: false,
            reason: "r".into(),
        };
        assert_eq!(flags(None, normal, Marks::default()), "");
        assert_eq!(
            flags(
                Some(&rejected),
                loud,
                Marks {
                    glitch: true,
                    hotwords: false
                }
            ),
            "sound_alike clipping glitch"
        );
        assert_eq!(flags(None, levels(&[]), Marks::default()), "very_quiet");
        let synonym = Outcome::DryRun {
            text: "navigate to seoul".into(),
            token: "T".into(),
            confidence: 0.95,
            sound_alike: true,
            synonym: true,
            filler: false,
        };
        assert_eq!(
            flags(Some(&synonym), normal, Marks::default()),
            "synonym sound_alike"
        );
        assert_eq!(
            flags(
                Some(&synonym),
                normal,
                Marks {
                    glitch: true,
                    hotwords: true
                }
            ),
            "synonym sound_alike hotwords glitch"
        );
        let filler = Outcome::Rejected {
            text: "uh hyperspaxe".into(),
            token: "T".into(),
            confidence: 0.86,
            sound_alike: false,
            synonym: false,
            filler: true,
            reason: "r".into(),
        };
        assert_eq!(flags(Some(&filler), normal, Marks::default()), "filler");
        let quiet = |peak_dbfs| Levels {
            peak_dbfs,
            rms_dbfs: -60.0,
        };
        assert_eq!(flags(None, quiet(-50.0), Marks::default()), "");
        assert_eq!(flags(None, quiet(-50.1), Marks::default()), "very_quiet");
    }
}
