//! Cloud engine: any service with an OpenAI-style `/audio/transcriptions` endpoint.

use std::fmt;
use std::time::Duration;

use anyhow::{anyhow, bail, Context};
use reqwest::blocking::{multipart, Client};
use reqwest::header::HeaderValue;

use crate::{wav, AudioClip, SttEngine, Transcript};

/// Longest part of an HTTP error body that goes into an error message.
const MAX_BODY_CHARS: usize = 200;

/// Settings of the cloud engine.
#[derive(Clone)]
pub struct OpenAiCompatConfig {
    /// Service address up to the API version, for example `https://api.openai.com/v1`.
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub language: Option<String>,
    pub timeout: Duration,
}

impl fmt::Debug for OpenAiCompatConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenAiCompatConfig")
            .field("base_url", &self.base_url)
            .field("api_key", &"***")
            .field("model", &self.model)
            .field("language", &self.language)
            .field("timeout", &self.timeout)
            .finish()
    }
}

/// Speech-to-text through an OpenAI-compatible service.
pub struct OpenAiCompatEngine {
    config: OpenAiCompatConfig,
    url: String,
    client: Client,
}

impl fmt::Debug for OpenAiCompatEngine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenAiCompatEngine")
            .field("config", &self.config)
            .finish()
    }
}

impl OpenAiCompatEngine {
    pub fn new(config: OpenAiCompatConfig) -> anyhow::Result<OpenAiCompatEngine> {
        if config.api_key.trim().is_empty() {
            bail!("api_key must not be empty");
        }
        // The error texts never show the key.
        if config.api_key.chars().any(char::is_control) {
            bail!("api_key has a control character (for example a line break)");
        }
        if HeaderValue::from_str(&format!("Bearer {}", config.api_key)).is_err() {
            bail!("api_key is not a valid HTTP header value");
        }
        if config.base_url.trim().is_empty() {
            bail!("base_url must not be empty");
        }
        if config.model.trim().is_empty() {
            bail!("model must not be empty");
        }
        let url = crate::request_url(&config.base_url);
        // The request must go to the origin that the key checks see.
        let origin = crate::url_origin(&config.base_url);
        if origin.is_none() || crate::url_origin(&url) != origin {
            bail!("base_url is not a valid http or https URL");
        }
        let client = Client::builder()
            .timeout(config.timeout)
            .build()
            .context("cannot create the HTTP client")?;
        Ok(OpenAiCompatEngine {
            config,
            url,
            client,
        })
    }
}

impl SttEngine for OpenAiCompatEngine {
    fn name(&self) -> &str {
        "openai-compatible"
    }

    fn transcribe(&mut self, clip: &AudioClip) -> anyhow::Result<Transcript> {
        if clip.samples.is_empty() {
            return Ok(Transcript {
                text: String::new(),
            });
        }
        let file = multipart::Part::bytes(wav::encode(clip))
            .file_name("audio.wav")
            .mime_str("audio/wav")
            .context("invalid audio type")?;
        let mut form = multipart::Form::new()
            .part("file", file)
            .text("model", self.config.model.clone())
            .text("response_format", "json");
        if let Some(language) = self.config.language.as_deref().filter(|l| !l.is_empty()) {
            form = form.text("language", language.to_string());
        }
        let response = self
            .client
            .post(&self.url)
            .bearer_auth(&self.config.api_key)
            .multipart(form)
            .send()
            .map_err(|e| anyhow!("transcription request failed: {}", describe(&e)))?;
        let status = response.status();
        let body = response
            .text()
            .map_err(|e| anyhow!("cannot read the reply (status {status}): {}", describe(&e)))?;
        if !status.is_success() {
            // Hide the key before the cut, so that no part of it stays.
            let body = body.replace(self.config.api_key.as_str(), "***");
            let short: String = body.chars().take(MAX_BODY_CHARS).collect();
            bail!("transcription service answered with status {status}: {short}");
        }
        let text = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| v.get("text").and_then(|t| t.as_str()).map(str::to_string))
            .ok_or_else(|| anyhow!("reply had no text field (status {status})"))?;
        Ok(Transcript {
            text: text.trim().to_string(),
        })
    }
}

/// Short reason for a failed request, without the URL.
fn describe(error: &reqwest::Error) -> &'static str {
    if error.is_timeout() {
        "timeout"
    } else if error.is_connect() {
        "cannot connect"
    } else {
        "network error"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    /// The inputs of the host tests, hostile spellings included.
    const BASE_URLS: &[&str] = &[
        "https://api.openai.com/v1",
        "https://API.OpenAI.com:443/v1/",
        "https://api.openai.com:8443/v1",
        "http://localhost:11434/v1",
        "http://[::1]:8080/v1",
        "https://good.example\\@evil.example/v1",
        "https://good.example@evil.example/v1",
        "https://user:pw@evil.example/v1",
        "https://good.ex\tample/v1",
        "https://good.ex\nample/v1",
        " https://good.example/v1 ",
        "https://b\u{fc}cher.example/v1",
        "https://%65vil.example/v1",
        "https://good.example./v1",
        "https://good.example?x=1",
        "https://good.example#frag",
        "https://good.example:/v1",
        "https://good.example",
        "HTTPS://GOOD.EXAMPLE",
    ];

    /// The origin of the URL that the HTTP client requests for a base URL
    /// is the origin that `url_origin` gives for the base URL. The request
    /// URL comes from the client itself.
    #[test]
    fn test_request_goes_to_the_origin_of_the_base_url() {
        let client = Client::new();
        for base in BASE_URLS {
            let expected = crate::url_origin(base);
            let request = client.post(crate::request_url(base)).build();
            let actual = request
                .ok()
                .and_then(|r| crate::url_origin(r.url().as_str()));
            assert_eq!(actual, expected, "{base:?}");
            // The engine refuses a base URL that the function cannot read.
            let engine = OpenAiCompatEngine::new(config(base.to_string(), None));
            assert_eq!(engine.is_ok(), expected.is_some(), "{base:?}");
        }
        assert_eq!(
            crate::url_origin("https://API.OpenAI.com:443/v1/").as_deref(),
            Some("https://api.openai.com")
        );
        assert_eq!(
            crate::url_origin("https://good.example\\@evil.example/v1").as_deref(),
            Some("https://good.example")
        );
    }

    struct Seen {
        method: String,
        path: String,
        auth: String,
        body: String,
    }

    /// Start a one-request server. `delay` is how long it waits before it answers.
    fn serve(status: u16, reply: &'static str, delay: Duration) -> (String, mpsc::Receiver<Seen>) {
        serve_with(status, move |_| reply.to_string(), delay)
    }

    /// As `serve`, but the reply is made from the `Authorization` header value.
    fn serve_with(
        status: u16,
        reply: impl FnOnce(&str) -> String + Send + 'static,
        delay: Duration,
    ) -> (String, mpsc::Receiver<Seen>) {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut request = match server.recv() {
                Ok(r) => r,
                Err(_) => return,
            };
            let mut body = Vec::new();
            std::io::Read::read_to_end(request.as_reader(), &mut body).ok();
            let auth = request
                .headers()
                .iter()
                .find(|h| h.field.equiv("Authorization"))
                .map(|h| h.value.to_string())
                .unwrap_or_default();
            let reply = reply(&auth);
            tx.send(Seen {
                method: request.method().to_string(),
                path: request.url().to_string(),
                auth,
                body: String::from_utf8_lossy(&body).into_owned(),
            })
            .ok();
            std::thread::sleep(delay);
            let response = tiny_http::Response::from_string(reply).with_status_code(status);
            request.respond(response).ok();
        });
        (format!("http://127.0.0.1:{port}"), rx)
    }

    fn config(base_url: String, language: Option<&str>) -> OpenAiCompatConfig {
        OpenAiCompatConfig {
            base_url,
            api_key: "test-key".into(),
            model: "whisper-test".into(),
            language: language.map(str::to_string),
            timeout: Duration::from_secs(5),
        }
    }

    fn clip() -> AudioClip {
        AudioClip {
            samples: vec![0.1; 1600],
        }
    }

    #[test]
    fn test_sends_the_documented_request() {
        for suffix in ["/v1", "/v1/"] {
            let (base, rx) = serve(200, r#"{"text":"ok"}"#, Duration::ZERO);
            let mut engine =
                OpenAiCompatEngine::new(config(format!("{base}{suffix}"), Some("de"))).unwrap();
            engine.transcribe(&clip()).unwrap();
            let seen = rx.recv_timeout(Duration::from_secs(2)).unwrap();
            assert_eq!(seen.method, "POST");
            assert_eq!(seen.path, "/v1/audio/transcriptions");
            assert_eq!(seen.auth, "Bearer test-key");
            for part in [
                "name=\"model\"",
                "whisper-test",
                "name=\"response_format\"",
                "RIFF",
                "name=\"language\"",
                "filename=\"audio.wav\"",
                "audio/wav",
            ] {
                assert!(seen.body.contains(part), "missing {part}");
            }
        }
        let (base, rx) = serve(200, r#"{"text":"ok"}"#, Duration::ZERO);
        let mut engine = OpenAiCompatEngine::new(config(format!("{base}/v1"), None)).unwrap();
        engine.transcribe(&clip()).unwrap();
        let seen = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(!seen.body.contains("name=\"language\""));
    }

    #[test]
    fn test_returns_the_text_field() {
        let (base, _rx) = serve(200, r#"{"text":" sail out for home "}"#, Duration::ZERO);
        let mut engine = OpenAiCompatEngine::new(config(base, None)).unwrap();
        assert_eq!(
            engine.transcribe(&clip()).unwrap().text,
            "sail out for home"
        );
    }

    #[test]
    fn test_http_error_is_an_error_with_the_status() {
        let (base, _rx) = serve(401, r#"{"error":"bad key"}"#, Duration::ZERO);
        let mut engine = OpenAiCompatEngine::new(config(base, None)).unwrap();
        let err = engine.transcribe(&clip()).unwrap_err().to_string();
        assert!(err.contains("401"), "{err}");
        assert!(!err.contains("test-key"), "{err}");
    }

    #[test]
    fn test_error_body_that_echoes_the_key_does_not_show_the_key() {
        let key = "sk-echo-secret-0123456789";
        // The key is near the start and across the 200-character cut.
        let (base, _rx) = serve_with(
            401,
            |auth| {
                let pad = "x".repeat(130);
                format!("{{\"error\":\"bad {auth}\",\"pad\":\"{pad}{auth}\"}}")
            },
            Duration::ZERO,
        );
        let mut cfg = config(base, None);
        cfg.api_key = key.to_string();
        let mut engine = OpenAiCompatEngine::new(cfg).unwrap();
        let err = format!("{:#}", engine.transcribe(&clip()).unwrap_err());
        assert!(err.contains("401"), "{err}");
        assert!(err.contains("***"), "{err}");
        assert!(!err.contains(key), "{err}");
        assert!(!err.contains("sk-echo"), "{err}");
    }

    #[test]
    fn test_key_with_control_character_is_rejected_at_construction() {
        for key in [
            "sk-abc\r\n",
            "sk-abc\n",
            "sk-\tabc",
            "sk-\u{7f}abc",
            "sk-\u{0}abc",
        ] {
            let mut cfg = config("http://127.0.0.1:1".into(), None);
            cfg.api_key = key.to_string();
            let err = format!("{:#}", OpenAiCompatEngine::new(cfg).unwrap_err());
            assert!(err.contains("api_key"), "{err}");
            assert!(!err.contains("abc"), "{err}");
        }
    }

    #[test]
    fn test_reply_without_text_is_an_error() {
        let (base, _rx) = serve(200, "{}", Duration::ZERO);
        let mut engine = OpenAiCompatEngine::new(config(base, None)).unwrap();
        let err = engine.transcribe(&clip()).unwrap_err().to_string();
        assert!(err.contains("text"), "{err}");
    }

    #[test]
    fn test_timeout_is_an_error() {
        let (base, _rx) = serve(200, r#"{"text":"late"}"#, Duration::from_secs(3));
        let mut cfg = config(base, None);
        cfg.timeout = Duration::from_millis(200);
        let mut engine = OpenAiCompatEngine::new(cfg).unwrap();
        let start = Instant::now();
        assert!(engine.transcribe(&clip()).is_err());
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn test_debug_hides_the_key() {
        let cfg = config("http://127.0.0.1:1".into(), None);
        let text = format!("{cfg:?}");
        assert!(!text.contains("test-key"), "{text}");
        assert!(text.contains("\"***\""), "{text}");
        let engine = OpenAiCompatEngine::new(cfg).unwrap();
        let text = format!("{engine:?}");
        assert!(!text.contains("test-key"), "{text}");
    }

    #[test]
    fn test_empty_key_is_rejected_at_construction() {
        let mut cfg = config("http://127.0.0.1:1".into(), None);
        cfg.api_key = String::new();
        let err = OpenAiCompatEngine::new(cfg).unwrap_err().to_string();
        assert!(err.contains("api_key"), "{err}");
    }

    #[test]
    fn test_empty_clip_makes_no_request() {
        let mut engine =
            OpenAiCompatEngine::new(config("http://127.0.0.1:1".into(), None)).unwrap();
        let out = engine.transcribe(&AudioClip { samples: vec![] }).unwrap();
        assert_eq!(out.text, "");
    }
}
