//! The pipeline from text to the request file, on the synthetic profile.

mod common;

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use common::{fixture_matcher, fixture_vocab, vocab};
use edcopilot_voicebridge_core::{Pipeline, Writer};

/// The fixture with its vocabulary files. `System1` has the real slot
/// shape: the name first, then alternates and a quay.
fn build_pipeline(request: PathBuf) -> Pipeline {
    let vocab = fixture_vocab();
    assert_eq!(vocab["System1"], ["Pellmar", "Pelmar", "Pellmar Quay"]);
    build_pipeline_with(request, vocab)
}

fn build_pipeline_with(request: PathBuf, vocab: HashMap<String, Vec<String>>) -> Pipeline {
    let matcher = fixture_matcher(vocab);
    let writer = Writer::with_timeout(
        request,
        Duration::from_millis(200),
        Duration::from_millis(10),
    )
    // No EDCoPilot runs in this test, so the token stays in the file.
    .confirm_timeout(None);
    Pipeline::new(matcher, writer)
}

#[test]
fn test_text_to_request_file() {
    let dir = tempfile::tempdir().unwrap();
    let request = dir.path().join("EDCoPilot.request.txt");
    let pipeline = build_pipeline(request.clone());

    let start = Instant::now();
    let hit = pipeline.handle_text("sail to Pellmar").unwrap().unwrap();
    eprintln!("handle_text on the fixture took {:?}", start.elapsed());
    assert_eq!(hit.confidence, 1.0);
    let token = Some(hit.token);
    assert_eq!(token.as_deref(), Some("HarbourAt:Pellmar:sail to"));
    assert_eq!(fs::read(&request).unwrap(), b"HarbourAt:Pellmar:sail to");

    // EDCoPilot consumes the file.
    fs::remove_file(&request).unwrap();
    assert_eq!(pipeline.handle_text("mmm gibberish").unwrap(), None);
    assert!(!request.exists());

    let token = pipeline
        .handle_text("help tide log")
        .unwrap()
        .map(|m| m.token);
    assert_eq!(token.as_deref(), Some("Guide:Panel:TideLog"));
    assert_eq!(fs::read(&request).unwrap(), b"Guide:Panel:TideLog");

    // A fixed destination: the command writes its own constant system name.
    fs::remove_file(&request).unwrap();
    let token = pipeline
        .handle_text("sail to the old mill")
        .unwrap()
        .map(|m| m.token);
    assert_eq!(token.as_deref(), Some("HarbourAt:Dunmore:sail to"));
    assert_eq!(fs::read(&request).unwrap(), b"HarbourAt:Dunmore:sail to");

    // A harbour query: the profile-level prefix grammar plus a slot.
    fs::remove_file(&request).unwrap();
    let token = pipeline
        .handle_text("tide at Pellmar")
        .unwrap()
        .map(|m| m.token);
    assert_eq!(
        token.as_deref(),
        Some("HarbourInfo:Pellmar:tide at Pellmar")
    );
    fs::remove_file(&request).unwrap();
    pipeline.handle_text("help tide log").unwrap();

    // A number command: the value is the number in the spoken phrase.
    fs::remove_file(&request).unwrap();
    let token = pipeline
        .handle_text("show buoys within 12 thousand yards")
        .unwrap()
        .map(|m| m.token);
    assert_eq!(token.as_deref(), Some("ShowBuoysWithin=12000"));
    fs::remove_file(&request).unwrap();
    pipeline.handle_text("help tide log").unwrap();

    // Token not consumed: the next send fails and keeps the first token.
    assert!(pipeline.handle_text("sail to Pellmar").is_err());
    assert_eq!(fs::read(&request).unwrap(), b"Guide:Panel:TideLog");

    // A quay spoken from the slot sends the harbour name (the first entry).
    fs::remove_file(&request).unwrap();
    let token = pipeline
        .handle_text("sail to Pellmar Quay")
        .unwrap()
        .map(|m| m.token);
    assert_eq!(token.as_deref(), Some("HarbourAt:Pellmar:sail to"));
    assert_eq!(fs::read(&request).unwrap(), b"HarbourAt:Pellmar:sail to");
}

fn token_of(pipeline: &Pipeline, text: &str) -> Option<String> {
    pipeline.match_text(text).map(|m| m.token)
}

#[test]
fn test_number_words_reach_number_commands() {
    let dir = tempfile::tempdir().unwrap();
    let pipeline = build_pipeline(dir.path().join("EDCoPilot.request.txt"));
    assert_eq!(
        token_of(&pipeline, "fog horn volume fifty five").as_deref(),
        Some("Prefs:HornVolume=\"55\"")
    );
    assert_eq!(
        token_of(&pipeline, "show buoys within twelve thousand yards").as_deref(),
        Some("ShowBuoysWithin=12000")
    );
}

#[test]
fn test_number_forms_of_speech_engines_reach_number_commands() {
    let dir = tempfile::tempdir().unwrap();
    let pipeline = build_pipeline(dir.path().join("EDCoPilot.request.txt"));
    let words = token_of(&pipeline, "show buoys within twelve thousand yards");
    assert_eq!(words.as_deref(), Some("ShowBuoysWithin=12000"));
    for text in [
        "show buoys within 12,000 yards",
        "show buoys within 12000 yards",
        "show buoys within 12.000 yards.",
    ] {
        assert_eq!(token_of(&pipeline, text), words, "{text}");
    }
    assert_eq!(
        token_of(&pipeline, "fog horn volume fifty-five").as_deref(),
        Some("Prefs:HornVolume=\"55\"")
    );
}

#[test]
fn test_number_word_in_a_name_still_matches() {
    let dir = tempfile::tempdir().unwrap();
    let vocab = vocab(&[("System1", &["Seven Gulls"])]);
    let pipeline = build_pipeline_with(dir.path().join("EDCoPilot.request.txt"), vocab);
    assert_eq!(
        token_of(&pipeline, "sail to seven gulls").as_deref(),
        Some("HarbourAt:Seven Gulls:sail to")
    );
}

#[test]
fn test_match_text_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let request = dir.path().join("EDCoPilot.request.txt");
    let pipeline = build_pipeline(request.clone());
    assert!(pipeline.match_text("sail to Pellmar").is_some());
    assert!(!request.exists());
}

#[test]
fn test_vocab_swap_through_pipeline() {
    let dir = tempfile::tempdir().unwrap();
    let pipeline = build_pipeline(dir.path().join("EDCoPilot.request.txt"));
    // Corran is in no vocabulary file and is no system of a fixed command.
    // (Dunmore is: "sail to Dunmore" says the command for the old mill.)
    assert!(pipeline.match_text("sail to Pellmar").is_some());
    assert_eq!(token_of(&pipeline, "sail to Corran"), None);
    pipeline.set_dynamic_vocab(HashMap::from([(
        "System1".to_string(),
        vec!["Corran".to_string()],
    )]));
    assert_eq!(token_of(&pipeline, "sail to Pellmar"), None);
    assert_eq!(
        token_of(&pipeline, "sail to Corran").as_deref(),
        Some("HarbourAt:Corran:sail to")
    );
    // The fixed system names stay after a vocabulary reload.
    assert_eq!(
        token_of(&pipeline, "sail to Dunmore").as_deref(),
        Some("HarbourAt:Dunmore:sail to")
    );
}

#[test]
fn test_send_match_writes_the_token_of_the_hit() {
    let dir = tempfile::tempdir().unwrap();
    let request = dir.path().join("EDCoPilot.request.txt");
    let pipeline = build_pipeline(request.clone());
    let hit = pipeline.match_text("sail to Pellmar").unwrap();
    // A vocabulary swap after the match does not change what is sent.
    pipeline.set_dynamic_vocab(HashMap::new());
    pipeline.send_match(&hit).unwrap();
    assert_eq!(fs::read(&request).unwrap(), b"HarbourAt:Pellmar:sail to");
}

#[test]
fn test_pipeline_is_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Pipeline>();
}
