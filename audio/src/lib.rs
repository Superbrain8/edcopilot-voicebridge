//! Microphone capture, utterance segmentation and the talk button.

pub mod capture;
pub mod convert;
pub mod keys;
pub mod segmenter;
pub mod talk_button;

pub use capture::{list_input_devices, select_device_name, Capture};
pub use convert::Resampler;
pub use segmenter::{EnergyVad, Segmenter, SegmenterConfig, Vad, FRAME_SAMPLES};
pub use talk_button::{
    spawn_talk_button, ButtonSource, EdgeDetector, InputScanner, JoystickButton, KeyboardKey,
    PressedInput, TalkButtonHandle, TalkEvent,
};

/// Sample rate of the audio that speech-to-text engines get, in Hz.
pub const SAMPLE_RATE: u32 = 16_000;
