//! Talk button: a keyboard key or a joystick button, polled on one thread.

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::{anyhow, Result};
use gilrs::Gilrs;

use crate::keys::parse_key_spec;

/// A change of the talk button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TalkEvent {
    Pressed,
    Released,
}

/// Turns a stream of "is it down" samples into `Pressed` / `Released` edges.
#[derive(Debug, Default)]
pub struct EdgeDetector {
    down: bool,
}

impl EdgeDetector {
    /// Starts in the "up" state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Gives an event only when the state changes.
    pub fn update(&mut self, down: bool) -> Option<TalkEvent> {
        if down == self.down {
            return None;
        }
        self.down = down;
        Some(if down {
            TalkEvent::Pressed
        } else {
            TalkEvent::Released
        })
    }
}

/// Something that can say if the talk button is down now.
pub trait ButtonSource: Send {
    fn is_down(&mut self) -> bool;
}

/// The Win32 calls for the keyboard and the side mouse buttons.
#[cfg(windows)]
mod win {
    use crate::keys::LayoutKey;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, MapVirtualKeyW, VkKeyScanW, MAPVK_VK_TO_CHAR,
    };

    /// The global state of the key, also while another window has the focus.
    pub fn is_down(vk: u8) -> bool {
        // SAFETY: GetAsyncKeyState takes any integer and has no other inputs.
        let state = unsafe { GetAsyncKeyState(i32::from(vk)) };
        (state as u16) & 0x8000 != 0
    }

    /// The key that types `c` on the active layout, with the modifiers that
    /// the character needs.
    pub fn vk_of_char(c: char) -> Option<LayoutKey> {
        let mut units = [0u16; 2];
        let [unit] = c.encode_utf16(&mut units) else {
            return None;
        };
        // SAFETY: VkKeyScanW takes any UTF-16 code unit and has no other inputs.
        let result = unsafe { VkKeyScanW(*unit) };
        if result == -1 {
            return None;
        }
        let [vk, shift_state] = (result as u16).to_le_bytes();
        (vk != 0 && vk != 0xFF).then_some(LayoutKey { vk, shift_state })
    }

    /// The character of the key on the active layout, without a dead key.
    pub fn char_of_vk(vk: u8) -> Option<char> {
        // SAFETY: MapVirtualKeyW takes any code and map type, it has no other inputs.
        let result = unsafe { MapVirtualKeyW(u32::from(vk), MAPVK_VK_TO_CHAR) };
        // The top bit marks a dead key.
        if result == 0 || result & 0x8000_0000 != 0 {
            return None;
        }
        char::from_u32(result & 0xFFFF)
    }
}

/// A keyboard key or a side mouse button. The state is read globally, so it
/// works while the game window has the focus.
pub struct KeyboardKey {
    #[cfg(windows)]
    vk: u8,
    #[cfg(not(windows))]
    key: device_query::Keycode,
    #[cfg(not(windows))]
    state: device_query::DeviceState,
}

impl KeyboardKey {
    /// Parses a key specification:
    ///
    /// - a key name, for example `F13`, `RControl`, `Space`, `Mouse4`, without
    ///   regard to case,
    /// - one character, for example `<`: the key that types it on the
    ///   active keyboard layout (Windows only),
    /// - a virtual-key code, `vk:226` or `vk:0xE2` (Windows only).
    pub fn parse(spec: &str) -> Result<KeyboardKey> {
        #[cfg(windows)]
        {
            Ok(KeyboardKey {
                vk: windows_vk(spec)?,
            })
        }
        #[cfg(not(windows))]
        {
            Ok(KeyboardKey {
                key: device_query_key(spec)?,
                state: device_query::DeviceState::new(),
            })
        }
    }

    /// Checks a key specification like `parse`, but opens no device. Gives
    /// the resolved virtual-key code on Windows, `None` on other platforms.
    pub fn check(spec: &str) -> Result<Option<u8>> {
        #[cfg(windows)]
        {
            windows_vk(spec).map(Some)
        }
        #[cfg(not(windows))]
        {
            device_query_key(spec).map(|_| None)
        }
    }
}

#[cfg(windows)]
fn windows_vk(spec: &str) -> Result<u8> {
    let parsed = parse_key_spec(spec).map_err(anyhow::Error::msg)?;
    crate::keys::resolve_vk(parsed, win::vk_of_char).map_err(anyhow::Error::msg)
}

#[cfg(not(windows))]
fn device_query_key(spec: &str) -> Result<device_query::Keycode> {
    use crate::keys::KeySpec;
    use std::str::FromStr;
    match parse_key_spec(spec).map_err(anyhow::Error::msg)? {
        KeySpec::Name(name) => device_query::Keycode::from_str(name)
            .map_err(|_| anyhow!("the key {name} works only on Windows")),
        KeySpec::Char(_) | KeySpec::Vk(_) => Err(anyhow!(
            "the key {spec:?} works only on Windows, use a key name such as F13"
        )),
    }
}

impl ButtonSource for KeyboardKey {
    #[cfg(windows)]
    fn is_down(&mut self) -> bool {
        win::is_down(self.vk)
    }

    #[cfg(not(windows))]
    fn is_down(&mut self) -> bool {
        use device_query::DeviceQuery;
        self.state.get_keys().contains(&self.key)
    }
}

/// A button of a joystick, HOTAS stick or other game controller.
///
/// `button` is the raw button code that `gilrs` reports for the device, not a
/// gamepad name such as "South". On Windows it is the index of the button in
/// the raw button list of the device, counted from 0. On Linux it is the evdev
/// button code. It is stable for one device on one platform. Run
/// `voicebridge inputs` while you hold the button: it gives the number that this
/// type takes.
///
/// The `gilrs` context is `Send` on all platforms, so it is made in `new`.
pub struct JoystickButton {
    device: String,
    button: u32,
    gilrs: Gilrs,
}

impl JoystickButton {
    /// `device` is a part of the device name (compared in lowercase); an empty
    /// `device` matches any device. Fails only if the OS joystick backend does
    /// not start. A device that is not connected is not an error.
    pub fn new(device: &str, button: u32) -> Result<JoystickButton> {
        let gilrs = Gilrs::new().map_err(|e| anyhow!("joystick backend failed to start: {e}"))?;
        Ok(JoystickButton {
            device: device.to_lowercase(),
            button,
            gilrs,
        })
    }
}

impl ButtonSource for JoystickButton {
    fn is_down(&mut self) -> bool {
        // `gilrs` updates its state from events.
        while self.gilrs.next_event().is_some() {}
        for (_, pad) in self.gilrs.gamepads() {
            if !pad.is_connected() || !pad.name().to_lowercase().contains(&self.device) {
                continue;
            }
            // Only the first matching device counts.
            return pad
                .state()
                .buttons()
                .any(|(code, data)| code.into_u32() == self.button && data.is_pressed());
        }
        false
    }
}

/// Stops the polling thread when dropped. No `Released` event is made for a
/// button that is down at that time.
pub struct TalkButtonHandle {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for TalkButtonHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Polls `source` every `poll` on one thread and calls `on_event` for each
/// edge.
pub fn spawn_talk_button(
    mut source: Box<dyn ButtonSource>,
    poll: Duration,
    mut on_event: impl FnMut(TalkEvent) + Send + 'static,
) -> TalkButtonHandle {
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let thread = std::thread::spawn(move || {
        let mut edges = EdgeDetector::new();
        while !flag.load(Ordering::SeqCst) {
            if let Some(event) = edges.update(source.is_down()) {
                on_event(event);
            }
            std::thread::sleep(poll);
        }
    });
    TalkButtonHandle {
        stop,
        thread: Some(thread),
    }
}

/// One key or joystick button that is down.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PressedInput {
    /// A key specification that `KeyboardKey::parse` accepts.
    Key(String),
    /// A device name and a button number that `JoystickButton::new` accepts.
    Joystick { device: String, button: u32 },
}

impl fmt::Display for PressedInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PressedInput::Key(spec) => write!(f, "key {spec}"),
            PressedInput::Joystick { device, button } => {
                write!(f, "joystick \"{device}\" button {button}")
            }
        }
    }
}

/// Reads the keys and joystick buttons that are down. It keeps one joystick
/// context for all scans.
pub struct InputScanner {
    /// `None`: the joystick backend did not start. Keys still work.
    gilrs: Option<Gilrs>,
}

impl InputScanner {
    /// Starts the joystick backend and waits 50 ms, so that the first scan
    /// sees the buttons that are already down.
    pub fn new() -> InputScanner {
        let gilrs = Gilrs::new().ok();
        if gilrs.is_some() {
            std::thread::sleep(Duration::from_millis(50));
        }
        InputScanner { gilrs }
    }

    /// The keys and joystick buttons that are down now. On Windows the keys
    /// come from a scan of all virtual-key codes. Each spec parses back to
    /// the same key. The left, right and middle mouse buttons are not in the
    /// list.
    pub fn scan(&mut self) -> Vec<PressedInput> {
        let mut out: Vec<PressedInput> =
            pressed_keys().into_iter().map(PressedInput::Key).collect();
        let Some(gilrs) = self.gilrs.as_mut() else {
            return out;
        };
        while gilrs.next_event().is_some() {}
        for (_, pad) in gilrs.gamepads() {
            if !pad.is_connected() {
                continue;
            }
            let mut numbers: Vec<u32> = pad
                .state()
                .buttons()
                .filter(|(_, data)| data.is_pressed())
                .map(|(code, _)| code.into_u32())
                .collect();
            numbers.sort_unstable();
            out.extend(numbers.into_iter().map(|button| PressedInput::Joystick {
                device: pad.name().to_string(),
                button,
            }));
        }
        out
    }
}

impl Default for InputScanner {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(windows)]
fn pressed_keys() -> Vec<String> {
    use crate::keys::{is_reported_vk, spec_for_vk};
    (1..=254u8)
        .filter(|vk| is_reported_vk(*vk) && win::is_down(*vk))
        .map(|vk| spec_for_vk(vk, win::char_of_vk, win::vk_of_char))
        .collect()
}

#[cfg(not(windows))]
fn pressed_keys() -> Vec<String> {
    use device_query::{DeviceQuery, DeviceState};
    let keys = std::panic::catch_unwind(|| DeviceState::new().get_keys()).unwrap_or_default();
    keys.iter().map(|k| k.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{mpsc, Arc};
    use std::time::Duration;

    #[test]
    fn test_edges() {
        let mut d = EdgeDetector::new();
        let got: Vec<_> = [false, true, true, false, false, true]
            .into_iter()
            .map(|s| d.update(s))
            .collect();
        assert_eq!(
            got,
            vec![
                None,
                Some(TalkEvent::Pressed),
                None,
                Some(TalkEvent::Released),
                None,
                Some(TalkEvent::Pressed)
            ]
        );
    }

    #[test]
    fn test_key_names() {
        assert!(KeyboardKey::parse("f13").is_ok());
        assert!(KeyboardKey::parse("F13").is_ok());
        assert!(KeyboardKey::parse("rcontrol").is_ok());
        let err = KeyboardKey::parse("NoSuchKey").err().unwrap().to_string();
        assert!(err.contains("NoSuchKey"), "{err}");
        assert!(err.contains("Space"), "{err}");
    }

    #[cfg(windows)]
    #[test]
    fn test_windows_key_forms() {
        for spec in ["vk:226", "vk:0xE2", "Mouse4", "mouse5", "RControl", "Grave"] {
            assert!(KeyboardKey::check(spec).is_ok(), "{spec}");
        }
        assert_eq!(KeyboardKey::check("vk:226").unwrap(), Some(226));
        assert_eq!(KeyboardKey::check("RControl").unwrap(), Some(0xA3));
        assert!(KeyboardKey::check("Mouse1").is_err());
        assert!(KeyboardKey::check("NumpadEnter").is_err());
        assert!(KeyboardKey::check(" ").is_err());
        assert!(
            KeyboardKey::check("\u{1F680}").is_err(),
            "no layout types a rocket"
        );
    }

    #[test]
    fn test_pressed_input_lines() {
        assert_eq!(PressedInput::Key("<".to_string()).to_string(), "key <");
        let j = PressedInput::Joystick {
            device: "MOZA AB9 Base".to_string(),
            button: 5,
        };
        assert_eq!(j.to_string(), "joystick \"MOZA AB9 Base\" button 5");
    }

    struct Scripted {
        states: Vec<bool>,
        calls: Arc<AtomicUsize>,
    }

    impl ButtonSource for Scripted {
        fn is_down(&mut self) -> bool {
            let i = self.calls.fetch_add(1, Ordering::SeqCst);
            self.states.get(i).copied().unwrap_or(false)
        }
    }

    #[test]
    fn test_spawn_reports_edges_and_stops_on_drop() {
        let calls = Arc::new(AtomicUsize::new(0));
        let source = Scripted {
            states: vec![false, true, true, false],
            calls: calls.clone(),
        };
        let (tx, rx) = mpsc::channel();
        let handle = spawn_talk_button(Box::new(source), Duration::from_millis(1), move |e| {
            let _ = tx.send(e);
        });
        let first = rx.recv_timeout(Duration::from_secs(1)).unwrap();
        let second = rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!((first, second), (TalkEvent::Pressed, TalkEvent::Released));
        drop(handle);
        let after_drop = calls.load(Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(calls.load(Ordering::SeqCst), after_drop);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn test_absent_joystick_is_not_down() {
        let mut j = JoystickButton::new("no such device", 0).unwrap();
        assert!(!j.is_down());
    }

    #[test]
    fn test_input_scanner_does_not_panic() {
        let mut scanner = InputScanner::new();
        let _ = scanner.scan();
        let _ = scanner.scan();
    }
}
