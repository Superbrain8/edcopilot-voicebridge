//! Microphone input stream that delivers 16 kHz mono samples.

use anyhow::{anyhow, bail, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, ErrorKind, SampleFormat, SizedSample, Stream, StreamConfig};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::convert::Resampler;

/// A running microphone stream. Dropping it stops the stream.
///
/// `cpal::Stream` is not `Send` on some platforms (for example Android and
/// older backends). A caller keeps `Capture` on the thread that made it.
pub struct Capture {
    _stream: Stream,
    glitches: Arc<AtomicU64>,
}

impl Capture {
    /// Opens an input device and starts the stream.
    ///
    /// `device` is a part of a device name, compared without regard to case.
    /// `None` selects the default input device. `on_samples` gets 16 kHz mono
    /// samples and runs on the audio thread: it must not block. `on_error`
    /// gets the text of a fatal stream error and runs on another thread.
    /// A non-fatal error (see `is_fatal`) only counts as a glitch.
    pub fn start(
        device: Option<&str>,
        on_samples: impl FnMut(&[f32]) + Send + 'static,
        on_error: impl FnMut(String) + Send + 'static,
    ) -> Result<Capture> {
        Capture::start_counted(device, Arc::new(AtomicU64::new(0)), on_samples, on_error)
    }

    /// As `start`, but the non-fatal stream errors count in `glitches`. The
    /// caller can read the counter on another thread.
    pub fn start_counted(
        device: Option<&str>,
        glitches: Arc<AtomicU64>,
        on_samples: impl FnMut(&[f32]) + Send + 'static,
        on_error: impl FnMut(String) + Send + 'static,
    ) -> Result<Capture> {
        let host = cpal::default_host();
        let available = device_names(&host);
        let chosen = select_device_name(device, &available).map_err(|e| anyhow!(e))?;
        let device = match chosen {
            Some(name) => find_device(&host, &name)?,
            None => host.default_input_device().ok_or_else(|| {
                anyhow!(
                    "no default input device exists; input devices: {}",
                    format_names(&available)
                )
            })?,
        };

        let name = device_name(&device).unwrap_or_else(|| "default input device".to_string());
        let supported = device
            .default_input_config()
            .with_context(|| failure_text("read the input configuration of", &name))?;
        let config = supported.config();
        let errors = ErrorSink {
            on_error,
            glitches: Arc::clone(&glitches),
        };
        let stream = match supported.sample_format() {
            SampleFormat::F32 => build::<f32>(&device, &name, config, |s| s, on_samples, errors),
            SampleFormat::I16 => build::<i16>(
                &device,
                &name,
                config,
                |s| f32::from(s) / 32_768.0,
                on_samples,
                errors,
            ),
            SampleFormat::U16 => build::<u16>(
                &device,
                &name,
                config,
                |s| (f32::from(s) - 32_768.0) / 32_768.0,
                on_samples,
                errors,
            ),
            other => bail!("unsupported input sample format: {other}"),
        }?;
        stream
            .play()
            .with_context(|| failure_text("start the stream of", &name))?;
        Ok(Capture {
            _stream: stream,
            glitches,
        })
    }

    /// The number of non-fatal stream errors (for example buffer overruns)
    /// since the start.
    pub fn glitches(&self) -> u64 {
        self.glitches.load(Ordering::Relaxed)
    }
}

/// `true` when a stream error of this kind stops the stream. A buffer
/// overrun or underrun and a refused real-time priority are not fatal: the
/// stream continues after them.
fn is_fatal(kind: ErrorKind) -> bool {
    !matches!(kind, ErrorKind::Xrun | ErrorKind::RealtimeDenied)
}

/// Where the stream errors go: fatal errors to the callback, others to the counter.
struct ErrorSink<E> {
    on_error: E,
    glitches: Arc<AtomicU64>,
}

/// Builds the input stream for sample type `T`.
fn build<T: SizedSample + Send + 'static>(
    device: &Device,
    name: &str,
    config: StreamConfig,
    to_f32: fn(T) -> f32,
    mut on_samples: impl FnMut(&[f32]) + Send + 'static,
    mut errors: ErrorSink<impl FnMut(String) + Send + 'static>,
) -> Result<Stream> {
    let mut resampler = Resampler::new(config.sample_rate, config.channels);
    let error_name = name.to_string();
    // Both buffers are reused, so the steady state does not allocate.
    let mut converted: Vec<f32> = Vec::new();
    let mut out: Vec<f32> = Vec::new();
    device
        .build_input_stream(
            config,
            move |data: &[T], _| {
                converted.clear();
                converted.extend(data.iter().map(|&s| to_f32(s)));
                out.clear();
                resampler.push(&converted, &mut out);
                if !out.is_empty() {
                    on_samples(&out);
                }
            },
            move |e: cpal::Error| {
                if is_fatal(e.kind()) {
                    (errors.on_error)(format!("{error_name}: {e}"));
                } else {
                    // No lock and no print: this can run on the audio thread.
                    errors.glitches.fetch_add(1, Ordering::Relaxed);
                }
            },
            None,
        )
        .with_context(|| failure_text("build a stream on", name))
}

/// Names of the input devices. Any host or device error gives fewer names.
pub fn list_input_devices() -> Vec<String> {
    device_names(&cpal::default_host())
}

fn device_name(device: &Device) -> Option<String> {
    device.description().ok().map(|d| d.name().to_string())
}

fn device_names(host: &cpal::Host) -> Vec<String> {
    match host.input_devices() {
        Ok(devices) => devices.filter_map(|d| device_name(&d)).collect(),
        Err(_) => Vec::new(),
    }
}

fn find_device(host: &cpal::Host, name: &str) -> Result<Device> {
    host.input_devices()
        .context("cannot list input devices")?
        .find(|d| device_name(d).as_deref() == Some(name))
        .ok_or_else(|| anyhow!("input device \"{name}\" is no longer available"))
}

/// Text for a start error: what failed and on which device.
fn failure_text(action: &str, device: &str) -> String {
    format!("cannot {action} input device \"{device}\"")
}

fn format_names(available: &[String]) -> String {
    if available.is_empty() {
        "none".to_string()
    } else {
        available.join(", ")
    }
}

/// Chooses an input device name from `available`.
///
/// `Ok(None)` means the default device (nothing requested). Otherwise an
/// exact match without regard to case wins, then the first name that
/// contains the requested text.
pub fn select_device_name(
    requested: Option<&str>,
    available: &[String],
) -> Result<Option<String>, String> {
    let wanted = match requested.map(str::trim) {
        None | Some("") => return Ok(None),
        Some(text) => text.to_lowercase(),
    };
    if available.is_empty() {
        return Err(format!(
            "input device \"{}\" not found: no input device exists",
            requested.unwrap_or_default().trim()
        ));
    }
    let lower: Vec<String> = available.iter().map(|n| n.trim().to_lowercase()).collect();
    let found = lower
        .iter()
        .position(|n| *n == wanted)
        .or_else(|| lower.iter().position(|n| n.contains(&wanted)));
    match found {
        Some(i) => Ok(Some(available[i].clone())),
        None => Err(format!(
            "input device \"{}\" not found; input devices: {}",
            requested.unwrap_or_default().trim(),
            format_names(available)
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn test_select_default_when_nothing_is_requested() {
        assert_eq!(select_device_name(None, &names(&["A", "B"])), Ok(None));
    }

    #[test]
    fn test_select_default_for_blank_request_or_no_devices() {
        assert_eq!(select_device_name(Some("  "), &names(&["A"])), Ok(None));
        assert_eq!(select_device_name(Some(""), &[]), Ok(None));
        assert_eq!(select_device_name(None, &[]), Ok(None));
    }

    #[test]
    fn test_select_by_part_of_the_name_without_case() {
        let available = names(&["Speakers", "Blue Yeti Mic"]);
        assert_eq!(
            select_device_name(Some("yeti"), &available),
            Ok(Some("Blue Yeti Mic".to_string()))
        );
        assert_eq!(
            select_device_name(Some("  YETI "), &available),
            Ok(Some("Blue Yeti Mic".to_string()))
        );
    }

    #[test]
    fn test_select_exact_match_wins_over_partial() {
        let available = names(&["Mic Array", "Mic"]);
        assert_eq!(
            select_device_name(Some("mic"), &available),
            Ok(Some("Mic".to_string()))
        );
    }

    #[test]
    fn test_select_first_partial_match_wins() {
        let available = names(&["USB Mic 1", "USB Mic 2"]);
        assert_eq!(
            select_device_name(Some("usb"), &available),
            Ok(Some("USB Mic 1".to_string()))
        );
    }

    #[test]
    fn test_select_unknown_device_names_the_choices() {
        let err = select_device_name(Some("nope"), &names(&["A", "B"])).unwrap_err();
        assert!(err.contains("nope"), "{err}");
        assert!(err.contains('A'), "{err}");
        assert!(err.contains('B'), "{err}");
    }

    #[test]
    fn test_select_with_no_devices() {
        let err = select_device_name(Some("x"), &[]).unwrap_err();
        assert!(err.contains("no input device"), "{err}");
    }

    #[test]
    fn test_failure_text_names_the_device() {
        assert_eq!(
            failure_text("start the stream of", "Blue Yeti"),
            "cannot start the stream of input device \"Blue Yeti\""
        );
    }

    #[test]
    fn test_xrun_and_realtime_denied_are_not_fatal() {
        assert!(!is_fatal(ErrorKind::Xrun));
        assert!(!is_fatal(ErrorKind::RealtimeDenied));
    }

    #[test]
    fn test_other_error_kinds_are_fatal() {
        for kind in [
            ErrorKind::DeviceBusy,
            ErrorKind::DeviceChanged,
            ErrorKind::DeviceNotAvailable,
            ErrorKind::HostUnavailable,
            ErrorKind::InvalidInput,
            ErrorKind::PermissionDenied,
            ErrorKind::ResourceExhausted,
            ErrorKind::StreamInvalidated,
            ErrorKind::UnsupportedConfig,
            ErrorKind::UnsupportedOperation,
            ErrorKind::BackendError,
            ErrorKind::Other,
        ] {
            assert!(is_fatal(kind), "{kind:?}");
        }
    }

    #[test]
    fn test_list_input_devices_does_not_panic() {
        let _ = list_input_devices();
    }

    #[test]
    #[ignore = "needs a microphone"]
    fn test_capture_gives_samples() {
        use std::sync::{Arc, Mutex};
        use std::time::Duration;

        let count = Arc::new(Mutex::new(0usize));
        let counter = Arc::clone(&count);
        let capture = Capture::start(
            None,
            move |samples| *counter.lock().unwrap() += samples.len(),
            |e| eprintln!("capture error: {e}"),
        )
        .expect("start capture");
        std::thread::sleep(Duration::from_secs(1));
        drop(capture);
        let n = *count.lock().unwrap();
        assert!((12_000..=20_000).contains(&n), "{n} samples");
    }
}
