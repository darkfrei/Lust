// the microphone of the window, as the page has it (web.rs): only while the world asks for it
// (gfx.mic, the line "mic" in the frame), and only the loudness, a number 0..1, ever leaves
// this file; the sound itself is never kept. cpal reads the microphone (WASAPI on Windows,
// ALSA on Linux, CoreAudio on macOS). the window shows that the microphone is on
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};

// the microphone goes off when the world has not asked for it this long (as the page)
pub const LET_GO: Duration = Duration::from_millis(1500);

// the loudness of a piece of sound: the root of the mean square, 0 for none
pub fn loudness<I: Iterator<Item = f32>>(samples: I) -> f32 {
    let (mut sum, mut n) = (0.0f64, 0u64);
    for s in samples {
        if s.is_finite() {
            sum += (s as f64) * (s as f64);
            n += 1;
        }
    }
    if n == 0 {
        0.0
    } else {
        (sum / n as f64).sqrt() as f32
    }
}

// the level the world gets, from the loudness, once a frame: speech is about 0.05 to 0.2 of
// the full scale, so it is made 5 times larger; it falls slowly (a tenth a frame) and rises at once
pub fn next_level(before: f32, loud: f32) -> f32 {
    (loud * 5.0).min(1.0).max(before * 0.9).clamp(0.0, 1.0)
}

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Off,
    On,
    // why it could not start (no microphone, blocked by the system); tried again after it
    // was let go
    Failed(String),
}

pub struct Mic {
    stream: Option<cpal::Stream>,
    // the last loudness the sound thread heard, as the bits of an f32
    heard: Arc<AtomicU32>,
    pub state: State,
    pub level: f32,
    asked: Option<Instant>,
}

impl Default for Mic {
    fn default() -> Mic {
        Mic { stream: None, heard: Arc::new(AtomicU32::new(0)), state: State::Off, level: 0.0, asked: None }
    }
}

fn listen<T>(device: &cpal::Device, config: cpal::StreamConfig, heard: Arc<AtomicU32>) -> Result<cpal::Stream, String>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    device
        .build_input_stream::<T, _, _>(
            config,
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                let loud = loudness(data.iter().map(|s| s.to_sample::<f32>()));
                heard.store(loud.to_bits(), Ordering::Relaxed);
            },
            |e| eprintln!("lust: microphone: {}", e),
            None,
        )
        .map_err(|e| e.to_string())
}

fn open(heard: Arc<AtomicU32>) -> Result<cpal::Stream, String> {
    let host = cpal::default_host();
    let device = host.default_input_device().ok_or("no microphone found")?;
    let supported = device.default_input_config().map_err(|e| e.to_string())?;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let stream = match format {
        SampleFormat::I8 => listen::<i8>(&device, config, heard)?,
        SampleFormat::I16 => listen::<i16>(&device, config, heard)?,
        SampleFormat::I32 => listen::<i32>(&device, config, heard)?,
        SampleFormat::U8 => listen::<u8>(&device, config, heard)?,
        SampleFormat::U16 => listen::<u16>(&device, config, heard)?,
        SampleFormat::F32 => listen::<f32>(&device, config, heard)?,
        SampleFormat::F64 => listen::<f64>(&device, config, heard)?,
        other => return Err(format!("the microphone gives {} samples", other)),
    };
    stream.play().map_err(|e| e.to_string())?;
    Ok(stream)
}

impl Mic {
    // once a frame: whether the frame asked for the microphone. starts it, or lets it go
    // LET_GO after the last ask
    pub fn want(&mut self, asked: bool, now: Instant) {
        if asked {
            self.asked = Some(now);
            if self.state == State::Off {
                self.heard.store(0, Ordering::Relaxed);
                match open(Arc::clone(&self.heard)) {
                    Ok(s) => {
                        self.stream = Some(s);
                        self.state = State::On;
                    }
                    Err(e) => {
                        eprintln!("lust: microphone: {}", e);
                        self.state = State::Failed(e);
                    }
                }
            }
        } else if self.state != State::Off && self.asked.is_none_or(|t| now.duration_since(t) > LET_GO) {
            self.stop();
        }
    }

    pub fn stop(&mut self) {
        self.stream = None;
        self.state = State::Off;
        self.level = 0.0;
    }

    // the level for this frame, 0..1 (0 while it is not on)
    pub fn read(&mut self) -> f32 {
        if self.state != State::On {
            return 0.0;
        }
        let loud = f32::from_bits(self.heard.load(Ordering::Relaxed));
        self.level = next_level(self.level, loud);
        self.level
    }

    // what the window writes in its corner, and the length of the bar 0..1
    pub fn note(&self) -> Option<(String, Option<f32>)> {
        match &self.state {
            State::Off => None,
            State::On => Some((String::from("microphone on"), Some(self.level))),
            State::Failed(e) => Some((format!("microphone: {}", e.chars().take(60).collect::<String>()), None)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cpal::Sample;

    #[test]
    fn the_loudness_of_sound() {
        assert_eq!(loudness([].into_iter()), 0.0);
        assert_eq!(loudness([0.5, -0.5, 0.5, -0.5].into_iter()), 0.5);
        assert_eq!(loudness([f32::NAN, 1.0].into_iter()), 1.0);
        // i16 as cpal turns it into f32: full scale is 1
        assert!((loudness([i16::MAX, i16::MIN].iter().map(|s| s.to_sample::<f32>())) - 1.0).abs() < 0.001);
    }

    #[test]
    fn the_level_rises_at_once_and_falls_slowly() {
        assert_eq!(next_level(0.0, 0.1), 0.5);
        assert_eq!(next_level(0.0, 0.5), 1.0, "never above 1");
        assert!((next_level(0.5, 0.0) - 0.45).abs() < 1e-6);
    }

    #[test]
    fn a_microphone_not_asked_for_stays_off_and_gives_zero() {
        let mut m = Mic::default();
        m.want(false, Instant::now());
        assert_eq!((m.state.clone(), m.read(), m.note()), (State::Off, 0.0, None));
        // a failed one is let go after LET_GO and may be tried again
        m.state = State::Failed(String::from("no microphone found"));
        let t = Instant::now();
        m.asked = Some(t);
        m.want(false, t + Duration::from_millis(100));
        assert!(matches!(m.state, State::Failed(_)), "not let go before LET_GO");
        assert_eq!(m.note().unwrap().0, "microphone: no microphone found");
        m.want(false, t + LET_GO + Duration::from_millis(1));
        assert_eq!(m.state, State::Off);
    }
}
