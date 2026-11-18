// words from the voice of the person (lust dev): the sound of the microphone (mic.rs) is
// turned into text by vosk (https://alphacephei.com/vosk, Apache 2.0) on this computer;
// nothing goes to the internet. vosk is not built into lust: lust opens its library while it
// runs (libvosk.dll on Windows, libvosk.so on Linux, libvosk.dylib on macOS) and a model of a
// language (a folder vosk-model-...), both beside lust (the model also one folder deeper). when
// they are not there lust works as before and the window says it hears no words.
// only text leaves this file: the phrases go to the AI (never into the world), the sound is
// never kept
use std::ffi::{c_char, c_float, c_int, c_short, c_void, CStr, CString};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use libloading::Library;

// vosk hears sound at this rate, one channel
pub const RATE: u32 = 16_000;
// how many pieces of sound wait for the recognizer at most (a piece is about 10 ms); more
// are dropped (a slow computer loses words, the window does not wait)
const QUEUE: usize = 400;

#[cfg(windows)]
const LIB: &str = "libvosk.dll";
#[cfg(target_os = "macos")]
const LIB: &str = "libvosk.dylib";
#[cfg(all(unix, not(target_os = "macos")))]
const LIB: &str = "libvosk.so";

type ModelNew = unsafe extern "C" fn(*const c_char) -> *mut c_void;
type Free = unsafe extern "C" fn(*mut c_void);
type RecNew = unsafe extern "C" fn(*mut c_void, c_float) -> *mut c_void;
type Accept = unsafe extern "C" fn(*mut c_void, *const c_short, c_int) -> c_int;
type Text = unsafe extern "C" fn(*mut c_void) -> *const c_char;
type LogLevel = unsafe extern "C" fn(c_int);

// ---------- where vosk is ----------

// the folders looked in: beside lust, and the folder voice beside it
pub fn places() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(d) = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())) {
        v.push(d.join("voice"));
        v.push(d);
    }
    v
}

// the folders vosk-model-... in a folder, and one level deeper (a zip unpacked into a folder
// of its own); not in the games
fn models_in(d: &Path, deeper: bool, out: &mut Vec<PathBuf>) {
    let Ok(list) = std::fs::read_dir(d) else { return };
    for e in list.flatten() {
        let (p, name) = (e.path(), e.file_name().to_string_lossy().to_string());
        if !p.is_dir() {
            continue;
        }
        if name.starts_with("vosk-model") {
            out.push(p);
        } else if deeper && name != "games" {
            models_in(&p, false, out);
        }
    }
}

// the library and the model in these folders; Err says what is missing, in words for a person
pub fn find(dirs: &[PathBuf]) -> Result<(PathBuf, PathBuf), String> {
    let lib = dirs.iter().map(|d| d.join(LIB)).find(|p| p.is_file());
    let mut models: Vec<PathBuf> = Vec::new();
    for d in dirs {
        models_in(d, true, &mut models);
    }
    models.sort();
    match (lib, models.into_iter().next()) {
        (Some(l), Some(m)) => Ok((l, m)),
        (None, _) => Err(format!("no voice: {} is not beside lust", LIB)),
        (_, None) => Err(String::from("no voice: no folder vosk-model-... beside lust (lust-voice-ru.zip)")),
    }
}

// the text of one field of an answer of vosk ({"text": "..."} or {"partial": "..."})
pub fn field(json: &str, key: &str) -> String {
    serde_json::from_str::<serde_json::Value>(json).ok().and_then(|v| v.get(key).and_then(|t| t.as_str()).map(|t| t.trim().to_string())).unwrap_or_default()
}

// ---------- the recognizer ----------

pub struct Engine {
    model: *mut c_void,
    rec: *mut c_void,
    accept: Accept,
    result: Text,
    partial: Text,
    last: Text,
    free_rec: Free,
    free_model: Free,
    // the library stays open while its functions are used
    _lib: Library,
}

// an Engine is used by one thread at a time (the thread of hearing, or a test)
unsafe impl Send for Engine {}

impl Engine {
    pub fn open(lib: &Path, model: &Path) -> Result<Engine, String> {
        // safety: opening the library runs its start code; it is vosk, chosen by the person
        // who put it beside lust. every function is taken by its name and its C type from
        // vosk_api.h (vosk 0.3.45)
        unsafe {
            let l = Library::new(lib).map_err(|e| format!("no voice: {} does not open: {}", LIB, e))?;
            let get = |name: &[u8]| -> Result<*const c_void, String> { l.get::<*const c_void>(name).map(|s| *s).map_err(|e| format!("no voice: {}: {}", String::from_utf8_lossy(name), e)) };
            let log: LogLevel = std::mem::transmute::<*const c_void, LogLevel>(get(b"vosk_set_log_level\0")?);
            let model_new: ModelNew = std::mem::transmute::<*const c_void, ModelNew>(get(b"vosk_model_new\0")?);
            let rec_new: RecNew = std::mem::transmute::<*const c_void, RecNew>(get(b"vosk_recognizer_new\0")?);
            let accept: Accept = std::mem::transmute::<*const c_void, Accept>(get(b"vosk_recognizer_accept_waveform_s\0")?);
            let result: Text = std::mem::transmute::<*const c_void, Text>(get(b"vosk_recognizer_result\0")?);
            let partial: Text = std::mem::transmute::<*const c_void, Text>(get(b"vosk_recognizer_partial_result\0")?);
            let last: Text = std::mem::transmute::<*const c_void, Text>(get(b"vosk_recognizer_final_result\0")?);
            let free_rec: Free = std::mem::transmute::<*const c_void, Free>(get(b"vosk_recognizer_free\0")?);
            let free_model: Free = std::mem::transmute::<*const c_void, Free>(get(b"vosk_model_free\0")?);
            // vosk says nothing on the console
            log(-1);
            let path = CString::new(model.to_string_lossy().as_bytes()).map_err(|_| String::from("no voice: a bad path of the model"))?;
            let m = model_new(path.as_ptr());
            if m.is_null() {
                return Err(format!("no voice: the model {} does not load", model.display()));
            }
            let r = rec_new(m, RATE as c_float);
            if r.is_null() {
                free_model(m);
                return Err(String::from("no voice: vosk makes no recognizer"));
            }
            Ok(Engine { model: m, rec: r, accept, result, partial, last, free_rec, free_model, _lib: l })
        }
    }

    fn text(&self, f: Text) -> String {
        // safety: vosk gives a text it owns, valid until the next call on the recognizer;
        // it is copied at once
        unsafe {
            let p = f(self.rec);
            if p.is_null() {
                String::new()
            } else {
                CStr::from_ptr(p).to_string_lossy().into_owned()
            }
        }
    }

    // sound in; a phrase when vosk heard one end (a pause after words)
    pub fn feed(&mut self, pcm: &[i16]) -> Option<String> {
        if pcm.is_empty() {
            return None;
        }
        let n = pcm.len().min(c_int::MAX as usize) as c_int;
        // safety: the pointer and the length are of one living slice of i16 (c short)
        let done = unsafe { (self.accept)(self.rec, pcm.as_ptr(), n) };
        if done == 1 {
            Some(field(&self.text(self.result), "text")).filter(|t| !t.is_empty())
        } else {
            None
        }
    }

    // the words of the phrase that is being said now
    pub fn partial(&self) -> String {
        field(&self.text(self.partial), "partial")
    }

    // what is left at the end of the sound
    pub fn finish(&mut self) -> Option<String> {
        Some(field(&self.text(self.last), "text")).filter(|t| !t.is_empty())
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        // safety: both were made by vosk in open and are freed once
        unsafe {
            (self.free_rec)(self.rec);
            (self.free_model)(self.model);
        }
    }
}

// ---------- sound to the rate of vosk ----------

// one channel of any rate to RATE as i16: an average over about one step (so the high sounds
// do not fold down), then a straight line between the two nearest samples
pub struct Resampler {
    step: f64,
    pos: f64,
    prev: f32,
    ring: Vec<f32>,
    at: usize,
    sum: f32,
}

impl Resampler {
    pub fn new(rate_in: u32) -> Resampler {
        let step = rate_in.max(1) as f64 / RATE as f64;
        let k = step.round().max(1.0) as usize;
        Resampler { step, pos: 1.0, prev: 0.0, ring: vec![0.0; k], at: 0, sum: 0.0 }
    }

    pub fn push(&mut self, input: &[f32]) -> Vec<i16> {
        let mut out = Vec::with_capacity((input.len() as f64 / self.step) as usize + 2);
        for &x in input {
            let x = if x.is_finite() { x } else { 0.0 };
            self.sum += x - self.ring[self.at];
            self.ring[self.at] = x;
            self.at = (self.at + 1) % self.ring.len();
            let cur = self.sum / self.ring.len() as f32;
            // pos is where the next sample out lies, after prev (0) up to cur (1)
            while self.pos <= 1.0 {
                let v = self.prev + (cur - self.prev) * self.pos as f32;
                out.push((v.clamp(-1.0, 1.0) * 32767.0) as i16);
                self.pos += self.step;
            }
            self.pos -= 1.0;
            self.prev = cur;
        }
        out
    }
}

// interleaved channels to one
pub fn mono(data: &[f32], channels: usize) -> Vec<f32> {
    let c = channels.max(1);
    data.chunks(c).map(|f| f.iter().sum::<f32>() / f.len() as f32).collect()
}

// ---------- hearing, on a thread of its own ----------

// what the window learns from the hearing thread
#[derive(Clone, Debug, Default)]
pub struct Heard {
    // "loading", "ready", or why there are no words ("no voice: ...")
    pub state: String,
    // the words being said now
    pub partial: String,
    // the phrases said, not yet taken by the window
    pub phrases: Vec<String>,
}

pub struct Hearing {
    pub tx: SyncSender<Vec<i16>>,
    pub out: Arc<Mutex<Heard>>,
}

fn hear(rx: Receiver<Vec<i16>>, out: Arc<Mutex<Heard>>, dirs: Vec<PathBuf>) {
    let set = |f: &dyn Fn(&mut Heard)| f(&mut out.lock().unwrap_or_else(|p| p.into_inner()));
    set(&|h| h.state = String::from("loading"));
    let mut engine = match find(&dirs).and_then(|(l, m)| Engine::open(&l, &m)) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("lust: {}", e);
            set(&|h| h.state = e.clone());
            return;
        }
    };
    set(&|h| h.state = String::from("ready"));
    loop {
        match rx.recv_timeout(Duration::from_millis(300)) {
            Ok(pcm) => {
                let phrase = engine.feed(&pcm);
                let partial = engine.partial();
                set(&|h| {
                    if let Some(p) = &phrase {
                        h.phrases.push(p.clone());
                    }
                    h.partial = partial.clone();
                });
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                if let Some(p) = engine.finish() {
                    set(&|h| h.phrases.push(p.clone()));
                }
                return;
            }
        }
    }
}

impl Hearing {
    // the recognizer starts on a thread of its own (loading a model takes a second or two)
    pub fn start() -> Hearing {
        let (tx, rx) = sync_channel::<Vec<i16>>(QUEUE);
        let out = Arc::new(Mutex::new(Heard::default()));
        let o = Arc::clone(&out);
        let dirs = places();
        std::thread::spawn(move || hear(rx, o, dirs));
        Hearing { tx, out }
    }

    // the new phrases, the words now, the state
    pub fn take(&self) -> (Vec<String>, String, String) {
        let mut h = self.out.lock().unwrap_or_else(|p| p.into_inner());
        (std::mem::take(&mut h.phrases), h.partial.clone(), h.state.clone())
    }
}

// ---------- a sound file (lust hear <file.wav>) ----------

// the samples of a wav file of whole numbers of 16 bits (any rate, any channels), as one
// channel 0..1, and its rate
pub fn read_wav(bytes: &[u8]) -> Result<(Vec<f32>, u32), String> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(String::from("not a wav file"));
    }
    let (mut at, mut fmt, mut data) = (12usize, None, None);
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let len = u32::from_le_bytes([bytes[at + 4], bytes[at + 5], bytes[at + 6], bytes[at + 7]]) as usize;
        let body = &bytes[at + 8..(at + 8 + len).min(bytes.len())];
        if id == b"fmt " && body.len() >= 16 {
            let tag = u16::from_le_bytes([body[0], body[1]]);
            let ch = u16::from_le_bytes([body[2], body[3]]) as usize;
            let rate = u32::from_le_bytes([body[4], body[5], body[6], body[7]]);
            let bits = u16::from_le_bytes([body[14], body[15]]);
            if tag != 1 || bits != 16 || ch == 0 {
                return Err(String::from("only wav files of 16 bit whole numbers"));
            }
            fmt = Some((ch, rate));
        } else if id == b"data" {
            data = Some(body);
        }
        at += 8 + len + (len & 1);
    }
    let ((ch, rate), data) = match (fmt, data) {
        (Some(f), Some(d)) => (f, d),
        _ => return Err(String::from("a wav file without fmt or data")),
    };
    let samples: Vec<f32> = data.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0).collect();
    Ok((mono(&samples, ch), rate))
}

// the phrases vosk hears in a sound file
pub fn hear_file(path: &Path) -> Result<Vec<String>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {}", path.display(), e))?;
    let (samples, rate) = read_wav(&bytes)?;
    let (lib, model) = find(&places())?;
    let mut e = Engine::open(&lib, &model)?;
    let mut r = Resampler::new(rate);
    let mut out = Vec::new();
    for piece in samples.chunks(1600) {
        if let Some(p) = e.feed(&r.push(piece)) {
            out.push(p);
        }
    }
    if let Some(p) = e.finish() {
        out.push(p);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sound_comes_to_the_rate_of_vosk() {
        // one second at 48000 is one second at 16000, in pieces of any size
        let mut r = Resampler::new(48_000);
        let tone: Vec<f32> = (0..48_000).map(|i| (i as f32 * 300.0 * std::f32::consts::TAU / 48_000.0).sin() * 0.5).collect();
        let mut n = 0;
        for piece in tone.chunks(441) {
            n += r.push(piece).len();
        }
        assert!((15_999..=16_001).contains(&n), "{}", n);
        // 16000 stays as it is
        let mut same = Resampler::new(16_000);
        assert_eq!(same.push(&[0.0, 0.5, -0.5]), vec![0, 16383, -16383]);
        // a tone of 300 Hz keeps its loudness
        let out = Resampler::new(48_000).push(&tone);
        let peak = out.iter().skip(100).map(|s| s.unsigned_abs()).max().unwrap();
        assert!((15_000..=16_500).contains(&peak), "{}", peak);
        assert_eq!(mono(&[1.0, 0.0, 0.5, 0.5], 2), vec![0.5, 0.5]);
    }

    #[test]
    fn a_wav_file_is_read() {
        let mut w = Vec::new();
        let data: Vec<u8> = [0i16, 16384, -16384, 0].iter().flat_map(|s| s.to_le_bytes()).collect();
        w.extend_from_slice(b"RIFF");
        w.extend_from_slice(&((36 + data.len()) as u32).to_le_bytes());
        w.extend_from_slice(b"WAVEfmt ");
        w.extend_from_slice(&16u32.to_le_bytes());
        w.extend_from_slice(&1u16.to_le_bytes());
        w.extend_from_slice(&2u16.to_le_bytes());
        w.extend_from_slice(&22_050u32.to_le_bytes());
        w.extend_from_slice(&(22_050u32 * 4).to_le_bytes());
        w.extend_from_slice(&4u16.to_le_bytes());
        w.extend_from_slice(&16u16.to_le_bytes());
        w.extend_from_slice(b"data");
        w.extend_from_slice(&(data.len() as u32).to_le_bytes());
        w.extend_from_slice(&data);
        let (s, rate) = read_wav(&w).unwrap();
        assert_eq!((s, rate), (vec![0.25, -0.25], 22_050));
        assert!(read_wav(b"nothing").is_err());
    }

    #[test]
    fn what_is_missing_is_said() {
        let d = std::env::temp_dir().join(format!("lust_voice_{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let e = find(std::slice::from_ref(&d)).unwrap_err();
        assert!(e.contains(LIB), "{}", e);
        std::fs::write(d.join(LIB), b"").unwrap();
        assert!(find(std::slice::from_ref(&d)).unwrap_err().contains("vosk-model"));
        std::fs::create_dir_all(d.join("lust-voice-ru/vosk-model-small-ru-0.22")).unwrap();
        let (l, m) = find(std::slice::from_ref(&d)).unwrap();
        assert!(l.ends_with(LIB) && m.ends_with("vosk-model-small-ru-0.22"));
        // a library that is not one does not open, and says so
        assert!(Engine::open(&l, &m).err().unwrap().starts_with("no voice"));
        assert_eq!(field("{\"text\" : \"\u{434}\u{430}\"}", "text"), "\u{434}\u{430}");
        assert_eq!(field("garbage", "text"), "");
        let _ = std::fs::remove_dir_all(&d);
    }
}
