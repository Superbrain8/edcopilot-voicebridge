#![cfg(feature = "local")]

use std::path::{Path, PathBuf};
use std::time::Instant;

use edcopilot_voicebridge_stt::{wav, ParakeetEngine, SttEngine};

/// The folder of the speech clips: `VOICEBRIDGE_STT_FIXTURES`, default
/// `private/stt_fixtures` in the workspace. A relative path starts at the
/// workspace folder. The repository has no recordings.
fn fixture(name: &str) -> PathBuf {
    let folder = std::env::var_os("VOICEBRIDGE_STT_FIXTURES")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("private/stt_fixtures"));
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(folder)
        .join(name)
}

fn normalise(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect()
}

// Needs the Parakeet model files: set VOICEBRIDGE_PARAKEET_DIR to the model directory.
// Needs the speech clips in VOICEBRIDGE_STT_FIXTURES: a clip that is absent
// gives a note on stderr, and the test skips that clip.
#[test]
#[ignore]
fn parakeet_transcribes_the_fixtures() {
    let dir = std::env::var("VOICEBRIDGE_PARAKEET_DIR").expect("VOICEBRIDGE_PARAKEET_DIR");
    let mut engine = ParakeetEngine::new(Path::new(&dir), 4, None).unwrap();
    let cases: [(&str, &[&str]); 3] = [
        ("en_set_course_home.wav", &["course", "home"]),
        ("en_music_volume_55.wav", &["volume"]),
        ("de_kurs_nach_hause.wav", &["kurs", "hause"]),
    ];
    for (file, words) in cases {
        let path = fixture(file);
        if !path.is_file() {
            eprintln!("skipped: no speech clip at {}", path.display());
            continue;
        }
        let clip = wav::read_file(&path).unwrap();
        let start = Instant::now();
        let text = engine.transcribe(&clip).unwrap().text;
        let took = start.elapsed();
        eprintln!("{file}: {text:?} in {:.3} s", took.as_secs_f64());
        let norm = normalise(&text);
        for word in words {
            assert!(norm.contains(word), "{file}: {text:?} lacks {word:?}");
        }
        assert!(took.as_secs_f64() < 3.0, "{file}: took {took:?}");
    }
}
