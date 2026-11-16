//! Local engine: NVIDIA Parakeet TDT through sherpa-onnx.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context};
use sherpa_onnx::{
    OfflineRecognizer, OfflineRecognizerConfig, OfflineStream, OfflineTransducerModelConfig,
};

use crate::hotwords::{self, Hotwords};
use crate::{AudioClip, SttEngine, Transcript, SAMPLE_RATE};

const MODEL_FILES: [&str; 4] = [
    "encoder.int8.onnx",
    "decoder.int8.onnx",
    "joiner.int8.onnx",
    "tokens.txt",
];

/// The beam width of `modified_beam_search`, as in the spike.
const MAX_ACTIVE_PATHS: i32 = 4;

/// Parakeet recognizers. They live as long as the engine.
///
/// `transcribe` always uses greedy search. With hotwords, the engine loads
/// the model a second time for `transcribe_biased`: `modified_beam_search`
/// with the hotwords. The beam search alone gave worse texts than greedy
/// search for real clips (empty texts and "Yeah."), so it is only a second
/// pass. The second text may confirm a command, never redirect it.
pub struct ParakeetEngine {
    greedy: OfflineRecognizer,
    biased: Option<Biased>,
}

/// The second recognizer and its hotword phrases, separated by `/`.
struct Biased {
    recognizer: OfflineRecognizer,
    hotwords: String,
}

impl ParakeetEngine {
    /// Loads the model. Without hotwords (`None` or an empty list) the
    /// engine has no second pass. With hotwords it loads the model a second
    /// time for `transcribe_biased`: `modified_beam_search` with 4 paths,
    /// and each stream gets the list.
    pub fn new(
        model_dir: &Path,
        threads: usize,
        hotwords: Option<&Hotwords>,
    ) -> anyhow::Result<ParakeetEngine> {
        if threads == 0 {
            bail!("threads must be 1 or more");
        }
        for file in MODEL_FILES {
            if !model_dir.join(file).is_file() {
                bail!(
                    "Parakeet model incomplete: {} is absent from {}",
                    file,
                    model_dir.display()
                );
            }
        }
        let path = |name: &str| Some(model_dir.join(name).to_string_lossy().into_owned());
        let mut config = OfflineRecognizerConfig::default();
        config.model_config.transducer = OfflineTransducerModelConfig {
            encoder: path(MODEL_FILES[0]),
            decoder: path(MODEL_FILES[1]),
            joiner: path(MODEL_FILES[2]),
        };
        config.model_config.tokens = path(MODEL_FILES[3]);
        config.model_config.model_type = Some("nemo_transducer".into());
        config.model_config.num_threads = i32::try_from(threads).context("too many threads")?;
        config.model_config.provider = Some("cpu".into());
        let load = |config: &OfflineRecognizerConfig| {
            OfflineRecognizer::create(config).with_context(|| {
                format!(
                    "Parakeet model could not be loaded from {}",
                    model_dir.display()
                )
            })
        };

        let biased = match hotwords.filter(|h| !h.is_empty()) {
            None => None,
            Some(h) => {
                let tokens_path = model_dir.join(MODEL_FILES[3]);
                let tokens = std::fs::read_to_string(&tokens_path)
                    .with_context(|| format!("cannot read {}", tokens_path.display()))?;
                let bad = hotwords::unencodable(&tokens, h.phrases());
                if !bad.is_empty() {
                    bail!(
                        "the model pieces cannot spell these hotwords: {}",
                        bad.join(", ")
                    );
                }
                // The vocabulary file is necessary only while the recognizer loads.
                let vocab = TempFile::write(&hotwords::bpe_vocab(&tokens))?;
                let mut beam = config.clone();
                beam.decoding_method = Some("modified_beam_search".into());
                beam.max_active_paths = MAX_ACTIVE_PATHS;
                beam.model_config.modeling_unit = Some("bpe".into());
                beam.model_config.bpe_vocab = Some(vocab.path.to_string_lossy().into_owned());
                beam.hotwords_score = h.score();
                Some(Biased {
                    recognizer: load(&beam)?,
                    hotwords: h.stream_text(),
                })
            }
        };
        Ok(ParakeetEngine {
            greedy: load(&config)?,
            biased,
        })
    }

    /// True when the engine has the second pass with hotwords.
    pub fn has_hotwords(&self) -> bool {
        self.biased.is_some()
    }
}

/// Decodes the clip with the stream of the recognizer.
fn decode(recognizer: &OfflineRecognizer, stream: OfflineStream, clip: &AudioClip) -> Transcript {
    stream.accept_waveform(SAMPLE_RATE as i32, &clip.samples);
    recognizer.decode(&stream);
    let text = stream.get_result().map(|r| r.text).unwrap_or_default();
    Transcript {
        text: text.trim().to_string(),
    }
}

/// A file in the temporary directory that is deleted on drop.
struct TempFile {
    path: PathBuf,
}

impl TempFile {
    fn write(text: &str) -> anyhow::Result<TempFile> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let name = format!("voicebridge-{}-{nanos}-bpe.vocab", std::process::id());
        let path = std::env::temp_dir().join(name);
        std::fs::write(&path, text).with_context(|| format!("cannot write {}", path.display()))?;
        Ok(TempFile { path })
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn empty() -> Transcript {
    Transcript {
        text: String::new(),
    }
}

impl SttEngine for ParakeetEngine {
    fn name(&self) -> &str {
        "parakeet"
    }

    fn transcribe(&mut self, clip: &AudioClip) -> anyhow::Result<Transcript> {
        if clip.samples.is_empty() {
            return Ok(empty());
        }
        Ok(decode(&self.greedy, self.greedy.create_stream(), clip))
    }

    fn transcribe_biased(&mut self, clip: &AudioClip) -> Option<anyhow::Result<Transcript>> {
        let biased = self.biased.as_ref()?;
        if clip.samples.is_empty() {
            return Some(Ok(empty()));
        }
        let stream = biased
            .recognizer
            .create_stream_with_hotwords(&biased.hotwords);
        Some(Ok(decode(&biased.recognizer, stream, clip)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The model directory of the tests that need the model. Set
    /// `VOICEBRIDGE_PARAKEET_DIR` to change it. `None`: the model is absent
    /// and the test skips.
    fn model_dir() -> Option<PathBuf> {
        let dir = std::env::var("VOICEBRIDGE_PARAKEET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                PathBuf::from(r"D:\Models\sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8")
            });
        if dir.join("tokens.txt").is_file() {
            Some(dir)
        } else {
            eprintln!("skipped: no Parakeet model in {}", dir.display());
            None
        }
    }

    /// A speech clip of the private folder `VOICEBRIDGE_STT_FIXTURES`
    /// (default `private/stt_fixtures` in the workspace; a relative path
    /// starts at the workspace folder). The repository has no recordings.
    /// `None`: the clip is absent and the test skips.
    fn fixture(name: &str) -> Option<AudioClip> {
        let folder = std::env::var_os("VOICEBRIDGE_STT_FIXTURES")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("private/stt_fixtures"));
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join(folder)
            .join(name);
        if !path.is_file() {
            eprintln!("skipped: no speech clip at {}", path.display());
            return None;
        }
        Some(crate::wav::read_file(&path).unwrap())
    }

    #[test]
    fn test_absent_model_dir_is_a_clear_error() {
        let dir = std::env::temp_dir().join("voicebridge-no-such-model-dir");
        let err = ParakeetEngine::new(&dir, 2, None).err().expect("must fail");
        let text = err.to_string();
        assert!(text.contains(&dir.display().to_string()), "{text}");
        assert!(text.contains("encoder.int8.onnx"), "{text}");
    }

    /// The pieces of the real model spell the shipped hotwords.
    #[test]
    fn test_model_pieces_spell_the_default_hotwords() {
        let Some(dir) = model_dir() else { return };
        let tokens = std::fs::read_to_string(dir.join("tokens.txt")).unwrap();
        let list = Path::new(env!("CARGO_MANIFEST_DIR")).join("../core/data/hotwords.txt");
        let h = Hotwords::load(&list, 1.5).unwrap();
        assert!(!h.is_empty());
        assert_eq!(
            hotwords::unencodable(&tokens, h.phrases()),
            Vec::<String>::new()
        );
        // A word in another script has no pieces in this model.
        let bad = vec!["\u{4e2d}\u{6587}".to_string()];
        assert_eq!(hotwords::unencodable(&tokens, &bad), bad);
    }

    #[test]
    fn test_unspellable_hotword_is_a_start_error() {
        let Some(dir) = model_dir() else { return };
        let h = Hotwords::parse("Sol\n\u{4e2d}\u{6587}\n", 1.5).unwrap();
        let err = ParakeetEngine::new(&dir, 1, Some(&h))
            .err()
            .expect("must fail");
        assert!(err.to_string().contains("\u{4e2d}\u{6587}"), "{err}");
    }

    /// The synthetic voice says "navigate to Sol". Greedy search hears
    /// "Seoul". The second pass with the hotword "Sol" fixes it, also after
    /// the engine deleted the vocabulary file.
    #[test]
    fn test_hotword_fixes_a_name() {
        let Some(dir) = model_dir() else { return };
        let Some(clip) = fixture("en_navigate_to_sol.wav") else {
            return;
        };
        let Some(home) = fixture("en_set_course_home.wav") else {
            return;
        };
        let mut plain = ParakeetEngine::new(&dir, 2, None).unwrap();
        let text = plain.transcribe(&clip).unwrap().text;
        assert!(text.contains("Seoul"), "{text}");
        assert!(plain.transcribe_biased(&clip).is_none());
        let h = Hotwords::parse("Sol\n", 1.5).unwrap();
        let mut engine = ParakeetEngine::new(&dir, 2, Some(&h)).unwrap();
        // The first pass stays greedy search.
        assert_eq!(engine.transcribe(&clip).unwrap().text, text);
        let text = engine.transcribe_biased(&clip).unwrap().unwrap().text;
        assert!(text.contains("Sol") && !text.contains("Seoul"), "{text}");
        let text = engine
            .transcribe_biased(&home)
            .unwrap()
            .unwrap()
            .text
            .to_lowercase();
        assert!(text.contains("course") && text.contains("home"), "{text}");
    }

    /// An empty list gives no second pass.
    #[test]
    fn test_empty_hotword_list_is_off() {
        let Some(dir) = model_dir() else { return };
        let empty = Hotwords::parse("", 1.5).unwrap();
        let plain = ParakeetEngine::new(&dir, 1, Some(&empty)).unwrap();
        assert!(!plain.has_hotwords());
    }
}
