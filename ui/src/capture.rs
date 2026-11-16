//! The capture of the talk button: the decision logic of `voicebridge bind`
//! without a write of the configuration.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use edcopilot_voicebridge_audio::PressedInput;
use voicebridge::bind::{Binding, Decision, ReleaseTracker};

/// The poll interval. `voicebridge bind` uses the same interval.
pub const CAPTURE_POLL: Duration = Duration::from_millis(20);
/// After this time without a complete press, the capture stops.
pub const CAPTURE_TIMEOUT: Duration = Duration::from_secs(30);

/// Waits for one complete press: one input goes down and up again. Inputs
/// that are down at the start do not count until their release. Several
/// inputs in one press do not count, and the capture waits for the next
/// press. `scan` gives the inputs that are down now.
pub fn capture_press(
    mut scan: impl FnMut() -> Vec<PressedInput>,
    cancel: &AtomicBool,
    poll: Duration,
    timeout: Duration,
) -> Result<Binding, String> {
    let start = Instant::now();
    let mut tracker = ReleaseTracker::new(scan());
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err("the capture was cancelled".to_string());
        }
        if start.elapsed() >= timeout {
            return Err(format!(
                "no key or button was pressed in {} seconds",
                timeout.as_secs()
            ));
        }
        std::thread::sleep(poll);
        match tracker.update(&scan()) {
            Decision::Pending | Decision::Several(_) => {}
            Decision::Cancel => return Err("the capture was cancelled with Esc".to_string()),
            Decision::Bind(input) => return Ok(Binding::from(input)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    fn k(spec: &str) -> PressedInput {
        PressedInput::Key(spec.to_string())
    }

    /// Gives the scans in order, then nothing pressed.
    fn script(scans: Vec<Vec<PressedInput>>) -> impl FnMut() -> Vec<PressedInput> {
        let mut scans: VecDeque<_> = scans.into();
        move || scans.pop_front().unwrap_or_default()
    }

    const FAST: Duration = Duration::from_millis(1);

    #[test]
    fn test_capture_gives_the_released_input() {
        let never = AtomicBool::new(false);
        // Enter is down at the start and does not count.
        let scans = vec![
            vec![k("Enter")],
            vec![k("Enter")],
            vec![],
            vec![k("F13")],
            vec![k("F13")],
            vec![],
        ];
        let binding = capture_press(script(scans), &never, FAST, CAPTURE_TIMEOUT).unwrap();
        assert_eq!(binding, Binding::Key("F13".into()));

        let moza = PressedInput::Joystick {
            device: "MOZA".into(),
            button: 5,
        };
        let scans = vec![vec![], vec![moza.clone()], vec![]];
        let binding = capture_press(script(scans), &never, FAST, CAPTURE_TIMEOUT).unwrap();
        assert_eq!(
            binding,
            Binding::Joystick {
                device: "MOZA".into(),
                button: 5
            }
        );
    }

    #[test]
    fn test_capture_waits_after_several_inputs() {
        let never = AtomicBool::new(false);
        let scans = vec![
            vec![],
            vec![k("LControl"), k("C")],
            vec![],
            vec![k("RControl")],
            vec![],
        ];
        let binding = capture_press(script(scans), &never, FAST, CAPTURE_TIMEOUT).unwrap();
        assert_eq!(binding, Binding::Key("RControl".into()));
    }

    #[test]
    fn test_escape_cancels() {
        let never = AtomicBool::new(false);
        let scans = vec![vec![], vec![k("Escape")], vec![]];
        let error = capture_press(script(scans), &never, FAST, CAPTURE_TIMEOUT).unwrap_err();
        assert!(error.contains("Esc"), "{error}");
    }

    #[test]
    fn test_capture_can_be_cancelled() {
        let cancel = std::sync::Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let worker = std::thread::spawn(move || {
            // Nothing is ever pressed.
            capture_press(Vec::new, &flag, FAST, CAPTURE_TIMEOUT)
        });
        std::thread::sleep(Duration::from_millis(30));
        cancel.store(true, Ordering::SeqCst);
        let error = worker.join().unwrap().unwrap_err();
        assert_eq!(error, "the capture was cancelled");
    }

    #[test]
    fn test_capture_times_out() {
        let never = AtomicBool::new(false);
        let error = capture_press(Vec::new, &never, FAST, Duration::from_millis(20)).unwrap_err();
        assert!(error.contains("no key or button"), "{error}");
    }
}
