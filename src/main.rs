use std::collections::VecDeque;
use std::fmt;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

use iced::mouse;
use iced::widget::canvas::{self, Frame, Path, Stroke};
use iced::widget::{Canvas, button, column, container, pick_list, row, slider, text};
use iced::{
    Color, ContentFit, Element, Length, Pixels, Point, Renderer, Subscription, Task, Theme,
    alignment,
};

use synthphone_e_vocal_dsp::audio::Waveform;
use synthphone_e_vocal_dsp::{MusicalSettings, ProcessingMode, VocalEffectsConfig};

mod audio;
mod mic;
mod midi;
mod realtime;

use audio::{AudioData, DspParams};
use mic::MicCapture;
use midi::MidiTimeline;
use realtime::{HALF, N, VisualizationSnapshot};

// ── Rodio output stream (thread-local — OutputStream is !Send) ────────────────
thread_local! {
    static AUDIO_OUT: std::cell::RefCell<Option<(
        rodio::OutputStream,
        rodio::OutputStreamHandle,
    )>> = const { std::cell::RefCell::new(None) };
}

// ── Colour palette ────────────────────────────────────────────────────────────
const PANEL: Color = Color { r: 0.08, g: 0.10, b: 0.14, a: 1.0 };
const GRID: Color = Color { r: 1.0, g: 1.0, b: 1.0, a: 0.06 };
const DIM: Color = Color { r: 0.45, g: 0.50, b: 0.55, a: 1.0 };
const GREEN: Color = Color { r: 0.27, g: 1.0, b: 0.53, a: 1.0 };
const CYAN: Color = Color { r: 0.27, g: 0.87, b: 1.0, a: 1.0 };
const ORANGE: Color = Color { r: 1.0, g: 0.53, b: 0.27, a: 1.0 };
const PURPLE: Color = Color { r: 0.80, g: 0.27, b: 1.0, a: 1.0 };
const FORMANT_COLOR: Color = Color { r: 0.3, g: 1.0, b: 0.4, a: 0.9 };
const BYPASS_COLOR: Color = Color { r: 1.0, g: 0.67, b: 0.0, a: 1.0 };

// ── Enums ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppMode {
    PitchCorrection,
    Vocode,
    Harmony,
    Dry,
}

impl fmt::Display for AppMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppMode::PitchCorrection => write!(f, "Pitch Correction"),
            AppMode::Vocode => write!(f, "Vocoder"),
            AppMode::Harmony => write!(f, "Harmony (MIDI)"),
            AppMode::Dry => write!(f, "Dry / Formant"),
        }
    }
}

const APP_MODES: [AppMode; 4] = [
    AppMode::PitchCorrection,
    AppMode::Vocode,
    AppMode::Harmony,
    AppMode::Dry,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AudioSource { File, Microphone }

impl fmt::Display for AudioSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AudioSource::File => write!(f, "File"),
            AudioSource::Microphone => write!(f, "Microphone"),
        }
    }
}

const AUDIO_SOURCES: [AudioSource; 2] = [AudioSource::File, AudioSource::Microphone];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CarrierWave { Sine, Saw, Square, Triangle }

impl fmt::Display for CarrierWave {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", match self {
            CarrierWave::Sine => "Sine",
            CarrierWave::Saw => "Saw",
            CarrierWave::Square => "Square",
            CarrierWave::Triangle => "Triangle",
        })
    }
}

impl From<CarrierWave> for Waveform {
    fn from(cw: CarrierWave) -> Waveform {
        match cw {
            CarrierWave::Sine => Waveform::Sine,
            CarrierWave::Saw => Waveform::Saw,
            CarrierWave::Square => Waveform::Square,
            CarrierWave::Triangle => Waveform::Triangle,
        }
    }
}

const CARRIER_WAVES: [CarrierWave; 4] = [
    CarrierWave::Sine, CarrierWave::Saw, CarrierWave::Square, CarrierWave::Triangle,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FormantMode { Off, Male, Female }

impl fmt::Display for FormantMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FormantMode::Off => write!(f, "Off"),
            FormantMode::Male => write!(f, "Male (↓)"),
            FormantMode::Female => write!(f, "Female (↑)"),
        }
    }
}

const FORMANT_MODES: [FormantMode; 3] = [FormantMode::Off, FormantMode::Male, FormantMode::Female];

// ── Entry point ───────────────────────────────────────────────────────────────

fn main() -> iced::Result {
    iced::application("SynthPhone DSP Viewer", App::update, App::view)
        .theme(|_| Theme::Dark)
        .subscription(App::subscription)
        .window(iced::window::Settings {
            size: iced::Size::new(1280.0, 860.0),
            min_size: Some(iced::Size::new(1000.0, 700.0)),
            ..Default::default()
        })
        .run_with(App::new)
}

// ── Application state ─────────────────────────────────────────────────────────

const MAX_SPEC_COLS: usize = 512;
const MAX_PITCH_HISTORY: usize = 512;
const BYPASS_SECS: f32 = 1.0;

struct App {
    // Audio source state
    audio_source: AudioSource,
    audio: Option<AudioData>,
    mic_capture: Option<MicCapture>,
    midi_timeline: Option<Arc<MidiTimeline>>,
    midi_filename: Option<String>,
    status: String,

    // Real-time shared state
    viz: Arc<Mutex<VisualizationSnapshot>>,
    dsp_params: Arc<Mutex<DspParams>>,
    dsp_stop: Arc<AtomicBool>,
    dsp_thread: Option<std::thread::JoinHandle<()>>,
    rodio_sink: Option<rodio::Sink>,

    // Current STFT frame for canvases
    frame_input: Vec<f32>,
    frame_output: Vec<f32>,
    frame_in_fft: Vec<f32>,
    frame_out_fft: Vec<f32>,
    frame_formant: Vec<f32>,
    frame_pitch_hz: f32,

    // Spectrogram columns (input and output separate)
    input_spec_cols: VecDeque<Vec<f32>>,
    output_spec_cols: VecDeque<Vec<f32>>,
    last_col_serial: u64,
    input_spec_img: iced::widget::image::Handle,
    output_spec_img: iced::widget::image::Handle,

    // Pitch history (input and output separate)
    input_pitch_history: VecDeque<f32>,
    output_pitch_history: VecDeque<f32>,

    // Transport
    play_position: usize,
    total_samples: usize,
    is_playing: bool,

    // Bypass
    bypass_until: Option<Instant>,

    // UI parameter state
    app_mode: AppMode,
    config: VocalEffectsConfig,
    pitch_strength: f32,
    octave: f32,
    carrier_wave: CarrierWave,
    carrier_freq: f32,
    formant_mode: FormantMode,
    formant_ratio: f32,
    bit_depth: f32,
    sample_rate_div: f32,
    harmony_strength: f32,
    harmony_octave: f32,
}

#[derive(Debug, Clone)]
enum Message {
    // File / source
    OpenFile,
    FileOpened(Option<std::path::PathBuf>),
    OpenMidi,
    MidiOpened(Option<std::path::PathBuf>),
    SourceChanged(AudioSource),
    // Transport
    PlayPause,
    Stop,
    Seek(f32),
    BypassNow,
    BypassTick,
    // Viz
    VizTick,
    // Mode
    ModeChanged(AppMode),
    // Pitch correction
    PitchStrengthChanged(f32),
    OctaveChanged(f32),
    // Vocoder
    CarrierWaveChanged(CarrierWave),
    CarrierFreqChanged(f32),
    // Formant shift
    FormantModeChanged(FormantMode),
    FormantRatioChanged(f32),
    // Bitcrusher
    BitDepthChanged(f32),
    SampleRateDivChanged(f32),
    // Harmony
    HarmonyStrengthChanged(f32),
    HarmonyOctaveChanged(f32),
}

impl App {
    fn new() -> (Self, Task<Message>) {
        let config = VocalEffectsConfig::default();
        let settings = MusicalSettings::default();
        let dsp_params = Arc::new(Mutex::new(DspParams {
            config: config.clone(),
            settings,
            bit_depth: 16,
            sample_rate_div: 1,
            carrier_freq: 110.0,
            carrier_waveform: Waveform::Saw,
            bypass_until: None,
        }));
        (
            Self {
                audio_source: AudioSource::File,
                audio: None,
                mic_capture: None,
                midi_timeline: None,
                midi_filename: None,
                status: "Open a WAV file to begin.".into(),
                viz: Arc::new(Mutex::new(VisualizationSnapshot::default())),
                dsp_params,
                dsp_stop: Arc::new(AtomicBool::new(false)),
                dsp_thread: None,
                rodio_sink: None,
                frame_input: vec![0.0; N],
                frame_output: vec![0.0; N],
                frame_in_fft: vec![0.0; HALF],
                frame_out_fft: vec![0.0; HALF],
                frame_formant: vec![0.0; HALF],
                frame_pitch_hz: 0.0,
                input_spec_cols: VecDeque::new(),
                output_spec_cols: VecDeque::new(),
                last_col_serial: 0,
                input_spec_img: empty_image(),
                output_spec_img: empty_image(),
                input_pitch_history: VecDeque::new(),
                output_pitch_history: VecDeque::new(),
                play_position: 0,
                total_samples: 0,
                is_playing: false,
                bypass_until: None,
                app_mode: AppMode::PitchCorrection,
                config,
                pitch_strength: 0.8,
                octave: 2.0,
                carrier_wave: CarrierWave::Saw,
                carrier_freq: 110.0,
                formant_mode: FormantMode::Off,
                formant_ratio: 1.0,
                bit_depth: 16.0,
                sample_rate_div: 1.0,
                harmony_strength: 0.8,
                harmony_octave: 2.0,
            },
            Task::none(),
        )
    }

    fn subscription(&self) -> Subscription<Message> {
        let viz = if self.audio.is_some() || self.audio_source == AudioSource::Microphone {
            iced::time::every(Duration::from_millis(33)).map(|_| Message::VizTick)
        } else {
            Subscription::none()
        };
        let bypass = if self.bypass_until.is_some() {
            iced::time::every(Duration::from_millis(100)).map(|_| Message::BypassTick)
        } else {
            Subscription::none()
        };
        Subscription::batch([viz, bypass])
    }

    fn build_dsp_params(&self) -> DspParams {
        let mut config = self.config.clone();
        let mut settings = MusicalSettings::default();

        settings.mode = match self.app_mode {
            AppMode::PitchCorrection => {
                config.pitch_correction_strength = self.pitch_strength;
                settings.octave = self.octave.round() as i8;
                ProcessingMode::PitchControl
            }
            AppMode::Vocode => ProcessingMode::Vocode,
            AppMode::Harmony => {
                config.pitch_correction_strength = self.harmony_strength;
                settings.octave = self.harmony_octave.round() as i8;
                ProcessingMode::Harmony
            }
            AppMode::Dry => {
                settings.octave = self.octave.round() as i8;
                ProcessingMode::Dry
            }
        };

        settings.formant = match self.formant_mode {
            FormantMode::Off => 0,
            FormantMode::Male => 1,
            FormantMode::Female => 2,
        };
        settings.formant_male_ratio = self.formant_ratio.min(1.0);
        settings.formant_female_ratio = self.formant_ratio.max(1.0);

        DspParams {
            config,
            settings,
            bit_depth: self.bit_depth.round() as i8,
            sample_rate_div: self.sample_rate_div.round() as i8,
            carrier_freq: self.carrier_freq,
            carrier_waveform: self.carrier_wave.into(),
            bypass_until: self.bypass_until,
        }
    }

    fn push_params(&self) {
        *self.dsp_params.lock().unwrap() = self.build_dsp_params();
    }

    fn clear_history(&mut self) {
        self.input_spec_cols.clear();
        self.output_spec_cols.clear();
        self.input_pitch_history.clear();
        self.output_pitch_history.clear();
        self.last_col_serial = 0;
    }

    fn stop_dsp(&mut self) {
        self.dsp_stop.store(true, Ordering::Relaxed);
        if let Some(s) = self.rodio_sink.take() { s.stop(); }
        if let Some(t) = self.dsp_thread.take() { let _ = t.join(); }
        self.dsp_stop = Arc::new(AtomicBool::new(false));
        self.is_playing = false;
    }

    fn ensure_rodio(&mut self) -> bool {
        let mut ok = false;
        AUDIO_OUT.with(|cell| {
            let mut opt = cell.borrow_mut();
            if opt.is_none() {
                match rodio::OutputStream::try_default() {
                    Ok((s, h)) => *opt = Some((s, h)),
                    Err(e) => self.status = format!("Audio init failed: {e}"),
                }
            }
            ok = opt.is_some();
        });
        ok
    }

    fn make_sink(&mut self) -> Option<rodio::Sink> {
        if !self.ensure_rodio() { return None; }
        AUDIO_OUT.with(|cell| {
            cell.borrow().as_ref().and_then(|(_, h)| rodio::Sink::try_new(h).ok())
        })
    }

    fn start_file_dsp(&mut self, from_sample: usize) {
        let (samples, sample_rate) = match self.audio {
            Some(ref a) => (a.samples.clone(), a.sample_rate),
            None => return,
        };
        self.stop_dsp();
        self.push_params();
        {
            let mut v = self.viz.lock().unwrap();
            v.play_position = from_sample;
            v.col_serial = 0;
        }
        self.last_col_serial = 0;

        let Some(sink) = self.make_sink() else { return };
        let midi = self.midi_timeline.clone();
        let (handle, source) = realtime::start_dsp_thread(
            samples, sample_rate, from_sample,
            Arc::clone(&self.dsp_params),
            Arc::clone(&self.viz),
            Arc::clone(&self.dsp_stop),
            midi,
        );
        sink.append(source);
        sink.play();
        self.rodio_sink = Some(sink);
        self.dsp_thread = Some(handle);
        self.is_playing = true;
    }

    fn start_mic_dsp(&mut self) {
        self.stop_dsp();
        self.push_params();

        // Open mic if not already open.
        if self.mic_capture.is_none() {
            match MicCapture::open() {
                Ok(mc) => {
                    self.config.sample_rate = mc.sample_rate as f32;
                    self.mic_capture = Some(mc);
                }
                Err(e) => {
                    self.status = format!("Mic error: {e}");
                    return;
                }
            }
        }

        let mic_ring = Arc::clone(&self.mic_capture.as_ref().unwrap().ring);
        let sample_rate = self.mic_capture.as_ref().unwrap().sample_rate;
        {
            let mut v = self.viz.lock().unwrap();
            v.play_position = 0;
            v.col_serial = 0;
            v.total_samples = 0;
        }
        self.last_col_serial = 0;

        let Some(sink) = self.make_sink() else { return };
        let midi = self.midi_timeline.clone();
        let (handle, source) = realtime::start_mic_dsp_thread(
            mic_ring, sample_rate,
            Arc::clone(&self.dsp_params),
            Arc::clone(&self.viz),
            Arc::clone(&self.dsp_stop),
            midi,
        );
        sink.append(source);
        sink.play();
        self.rodio_sink = Some(sink);
        self.dsp_thread = Some(handle);
        self.is_playing = true;
        self.status = "Microphone live.".into();
    }

    fn update(&mut self, msg: Message) -> Task<Message> {
        match msg {
            // ── Source ────────────────────────────────────────────────────────
            Message::SourceChanged(src) => {
                if src == self.audio_source { return Task::none(); }
                self.stop_dsp();
                self.clear_history();
                self.audio_source = src;
                match src {
                    AudioSource::File => {
                        self.status = if self.audio.is_some() {
                            "File loaded. Press Play.".into()
                        } else {
                            "Open a WAV file to begin.".into()
                        };
                    }
                    AudioSource::Microphone => {
                        self.start_mic_dsp();
                    }
                }
            }

            // ── File loading ──────────────────────────────────────────────────
            Message::OpenFile => {
                return Task::perform(
                    async {
                        rfd::AsyncFileDialog::new()
                            .add_filter("WAV Audio", &["wav", "WAV"])
                            .pick_file()
                            .await
                            .map(|f| f.path().to_path_buf())
                    },
                    Message::FileOpened,
                );
            }
            Message::FileOpened(Some(path)) => match audio::load_wav(&path) {
                Ok(data) => {
                    self.config.sample_rate = data.sample_rate as f32;
                    self.total_samples = data.samples.len();
                    self.status = format!(
                        "{}  ·  {}Hz  ·  {} samples",
                        path.file_name().unwrap_or_default().to_string_lossy(),
                        data.sample_rate,
                        data.samples.len(),
                    );
                    self.stop_dsp();
                    self.play_position = 0;
                    self.clear_history();
                    self.audio = Some(data);
                    self.audio_source = AudioSource::File;
                }
                Err(e) => self.status = format!("Error loading file: {e}"),
            },
            Message::FileOpened(None) => {}

            // ── MIDI loading ──────────────────────────────────────────────────
            Message::OpenMidi => {
                return Task::perform(
                    async {
                        rfd::AsyncFileDialog::new()
                            .add_filter("MIDI", &["mid", "midi", "MID", "MIDI"])
                            .pick_file()
                            .await
                            .map(|f| f.path().to_path_buf())
                    },
                    Message::MidiOpened,
                );
            }
            Message::MidiOpened(Some(path)) => {
                let sr = self.config.sample_rate as u32;
                match MidiTimeline::load(&path, sr) {
                    Ok(tl) => {
                        self.midi_filename = path.file_name()
                            .map(|n| n.to_string_lossy().into_owned());
                        self.midi_timeline = Some(Arc::new(tl));
                        self.status = format!(
                            "MIDI loaded: {}",
                            self.midi_filename.as_deref().unwrap_or("?")
                        );
                        // Push new timeline into running DSP thread if playing.
                        if self.is_playing {
                            self.push_params();
                        }
                    }
                    Err(e) => self.status = format!("MIDI error: {e}"),
                }
            }
            Message::MidiOpened(None) => {}

            // ── Transport ─────────────────────────────────────────────────────
            Message::PlayPause => {
                if self.audio_source == AudioSource::Microphone {
                    if self.is_playing {
                        self.stop_dsp();
                    } else {
                        self.start_mic_dsp();
                    }
                } else if self.is_playing {
                    self.stop_dsp();
                    self.play_position = self.viz.lock().unwrap().play_position;
                } else {
                    self.start_file_dsp(self.play_position);
                }
            }
            Message::Stop => {
                self.stop_dsp();
                self.play_position = 0;
                self.clear_history();
            }
            Message::Seek(frac) => {
                let new_pos = (frac * self.total_samples as f32) as usize;
                self.play_position = new_pos;
                self.clear_history();
                if self.is_playing {
                    self.start_file_dsp(new_pos);
                }
            }

            // ── Bypass ────────────────────────────────────────────────────────
            Message::BypassNow => {
                let until = Instant::now() + Duration::from_secs_f32(BYPASS_SECS);
                self.bypass_until = Some(until);
                self.dsp_params.lock().unwrap().bypass_until = Some(until);
            }
            Message::BypassTick => {
                if let Some(t) = self.bypass_until {
                    if Instant::now() >= t {
                        self.bypass_until = None;
                        self.dsp_params.lock().unwrap().bypass_until = None;
                    }
                }
            }

            // ── Visualization tick ────────────────────────────────────────────
            Message::VizTick => {
                let (col_serial, play_pos, total, pitch_hz, out_pitch_hz,
                     in_spec_col, out_spec_col) = {
                    let v = self.viz.lock().unwrap();
                    (v.col_serial, v.play_position, v.total_samples,
                     v.pitch_hz, v.output_pitch_hz,
                     v.input_spec_col.clone(), v.output_spec_col.clone())
                };

                if col_serial > self.last_col_serial {
                    let v = self.viz.lock().unwrap();
                    // Snapshot frame data
                    self.frame_input = v.input_window.clone();
                    self.frame_output = v.output_window.clone();
                    self.frame_in_fft = v.input_fft.clone();
                    self.frame_out_fft = v.output_fft.clone();
                    self.frame_formant = v.formant_envelope.clone();
                    self.frame_pitch_hz = v.pitch_hz;
                    self.last_col_serial = col_serial;
                    drop(v);

                    // Accumulate spectrogram columns
                    self.input_spec_cols.push_back(in_spec_col);
                    if self.input_spec_cols.len() > MAX_SPEC_COLS {
                        self.input_spec_cols.pop_front();
                    }
                    self.output_spec_cols.push_back(out_spec_col);
                    if self.output_spec_cols.len() > MAX_SPEC_COLS {
                        self.output_spec_cols.pop_front();
                    }
                }

                self.play_position = play_pos;
                self.total_samples = total.max(self.total_samples);

                self.input_pitch_history.push_back(pitch_hz);
                if self.input_pitch_history.len() > MAX_PITCH_HISTORY {
                    self.input_pitch_history.pop_front();
                }
                self.output_pitch_history.push_back(out_pitch_hz);
                if self.output_pitch_history.len() > MAX_PITCH_HISTORY {
                    self.output_pitch_history.pop_front();
                }

                // End of file → loop
                if self.audio_source == AudioSource::File
                    && self.is_playing && total > 0 && play_pos >= total
                {
                    self.clear_history();
                    self.start_file_dsp(0);
                }

                // Rebuild spectrogram images
                let in_cols: Vec<Vec<f32>> = self.input_spec_cols.iter().cloned().collect();
                let out_cols: Vec<Vec<f32>> = self.output_spec_cols.iter().cloned().collect();
                self.input_spec_img = build_spectrogram_image(&in_cols, self.config.sample_rate);
                self.output_spec_img = build_spectrogram_image(&out_cols, self.config.sample_rate);
            }

            // ── Parameter changes ─────────────────────────────────────────────
            Message::ModeChanged(m) => { self.app_mode = m; self.push_params(); }
            Message::PitchStrengthChanged(v) => { self.pitch_strength = v; self.push_params(); }
            Message::OctaveChanged(v) => { self.octave = v; self.push_params(); }
            Message::CarrierWaveChanged(w) => { self.carrier_wave = w; self.push_params(); }
            Message::CarrierFreqChanged(v) => { self.carrier_freq = v; self.push_params(); }
            Message::FormantModeChanged(m) => { self.formant_mode = m; self.push_params(); }
            Message::FormantRatioChanged(v) => { self.formant_ratio = v; self.push_params(); }
            Message::BitDepthChanged(v) => { self.bit_depth = v; self.push_params(); }
            Message::SampleRateDivChanged(v) => { self.sample_rate_div = v; self.push_params(); }
            Message::HarmonyStrengthChanged(v) => { self.harmony_strength = v; self.push_params(); }
            Message::HarmonyOctaveChanged(v) => { self.harmony_octave = v; self.push_params(); }
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        let sr = self.config.sample_rate;
        let play_frac = if self.total_samples > 0 {
            self.play_position as f32 / self.total_samples as f32
        } else {
            0.0
        };

        // ── Header (mode + source pickers) ────────────────────────────────────
        let header = column![
            text("Mode").size(13),
            pick_list(&APP_MODES[..], Some(self.app_mode), Message::ModeChanged).width(Length::Fill),
            text("Source").size(13),
            pick_list(&AUDIO_SOURCES[..], Some(self.audio_source), Message::SourceChanged).width(Length::Fill),
        ]
        .spacing(6);

        // ── Mode-specific controls ────────────────────────────────────────────
        let mode_controls: Element<'_, Message> = match self.app_mode {
            AppMode::PitchCorrection => column![
                text("Correction Strength").size(13),
                slider(0.0..=1.0_f32, self.pitch_strength, Message::PitchStrengthChanged).step(0.01_f32),
                text(format!("{:.2}", self.pitch_strength)).size(12),
                text("Pitch Shift").size(13),
                slider(1.0_f32..=8.0_f32, self.octave, Message::OctaveChanged).step(0.1_f32),
                text(format!("{:.1}× ({:.1}×)", self.octave, self.octave * 0.5)).size(12),
            ].spacing(6).into(),

            AppMode::Vocode => column![
                text("Carrier Waveform").size(13),
                pick_list(&CARRIER_WAVES[..], Some(self.carrier_wave), Message::CarrierWaveChanged),
                text("Carrier Freq (Hz)").size(13),
                slider(50.0_f32..=2000.0_f32, self.carrier_freq, Message::CarrierFreqChanged).step(1.0_f32),
                text(format!("{:.0} Hz", self.carrier_freq)).size(12),
            ].spacing(6).into(),

            AppMode::Harmony => column![
                text("MIDI file").size(13),
                text(self.midi_filename.as_deref().unwrap_or("(none)")).size(11),
                button("Open MIDI…").on_press(Message::OpenMidi),
                text("Correction Strength").size(13),
                slider(0.0..=1.0_f32, self.harmony_strength, Message::HarmonyStrengthChanged).step(0.01_f32),
                text(format!("{:.2}", self.harmony_strength)).size(12),
                text("Pitch Shift").size(13),
                slider(1.0_f32..=8.0_f32, self.harmony_octave, Message::HarmonyOctaveChanged).step(0.1_f32),
                text(format!("{:.1}×", self.harmony_octave)).size(12),
            ].spacing(6).into(),

            AppMode::Dry => column![
                text("Pitch Shift").size(13),
                slider(1.0_f32..=8.0_f32, self.octave, Message::OctaveChanged).step(0.1_f32),
                text(format!("{:.1}× ({:.1}×)", self.octave, self.octave * 0.5)).size(12),
            ].spacing(6).into(),
        };

        // ── Global effects ────────────────────────────────────────────────────
        let effects_controls = column![
            text("── Effects ──").size(12),
            text("Formant").size(13),
            pick_list(&FORMANT_MODES[..], Some(self.formant_mode), Message::FormantModeChanged),
            slider(0.25_f32..=4.0_f32, self.formant_ratio, Message::FormantRatioChanged).step(0.05_f32),
            text(format!("Ratio {:.2}×", self.formant_ratio)).size(12),
            text("Bit Depth").size(13),
            slider(1.0_f32..=16.0_f32, self.bit_depth, Message::BitDepthChanged).step(1.0_f32),
            text(format!("{} bit", self.bit_depth as i8)).size(12),
            text("Sample Rate ÷").size(13),
            slider(1.0_f32..=32.0_f32, self.sample_rate_div, Message::SampleRateDivChanged).step(1.0_f32),
            text(format!("÷{}", self.sample_rate_div as i8)).size(12),
        ].spacing(4);

        let controls = column![
            button("Open WAV file").on_press(Message::OpenFile),
            header,
            mode_controls,
            effects_controls,
        ]
        .spacing(12)
        .padding(16)
        .width(240);

        // ── Transport + bypass ────────────────────────────────────────────────
        let has_audio = self.audio.is_some() || self.audio_source == AudioSource::Microphone;
        let play_label = if self.is_playing { "Pause" } else { "Play" };
        let is_file = self.audio_source == AudioSource::File;

        let bypass_label = if let Some(until) = self.bypass_until {
            let remaining = (until - Instant::now()).as_secs_f32().max(0.0);
            format!("Bypassing {:.1}s", remaining)
        } else {
            "Bypass Now".into()
        };

        let bypass_color = if self.bypass_until.is_some() { BYPASS_COLOR } else { Color::WHITE };
        let bypass_btn = if has_audio && self.is_playing {
            button(text(bypass_label).color(bypass_color)).on_press(Message::BypassNow)
        } else {
            button(text(bypass_label))
        };

        let mut transport_row = row![
            if has_audio {
                button(play_label).on_press(Message::PlayPause)
            } else {
                button(play_label)
            },
        ].spacing(8).padding([4, 0]);

        if is_file {
            transport_row = transport_row.push(
                if has_audio { button("Stop").on_press(Message::Stop) } else { button("Stop") }
            );
            transport_row = transport_row.push(
                slider(0.0_f32..=1.0_f32, play_frac, Message::Seek)
                    .step(0.0005_f32)
                    .width(Length::Fill)
            );
            transport_row = transport_row.push(
                text(format!(
                    "{} / {}",
                    fmt_time(self.play_position, sr as u32),
                    fmt_time(self.total_samples, sr as u32),
                )).size(12)
            );
        } else {
            // Mic mode: just a spacer and live indicator
            transport_row = transport_row.push(
                text(if self.is_playing { "● LIVE" } else { "○ Stopped" }).size(12)
            );
        }

        transport_row = transport_row.push(bypass_btn);

        let out_label = format!("Output  ({})", self.app_mode);

        // ── Waveform + spectrum panels ────────────────────────────────────────
        let panels = row![
            column![
                text("Input  (current block)").size(12),
                Canvas::new(WaveformView { samples: self.frame_input.clone(), color: GREEN })
                    .width(Length::Fill).height(90),
                text("Input · Spectrum + formant envelope").size(12),
                Canvas::new(SpectrumView {
                    spectrum: self.frame_in_fft.clone(),
                    formant_env: self.frame_formant.clone(),
                    color: ORANGE,
                    sample_rate: sr,
                }).width(Length::Fill).height(180),
            ].spacing(3).width(Length::FillPortion(1)),
            column![
                text(out_label).size(12),
                Canvas::new(WaveformView { samples: self.frame_output.clone(), color: CYAN })
                    .width(Length::Fill).height(90),
                text("Output · Spectrum + formant envelope").size(12),
                Canvas::new(SpectrumView {
                    spectrum: self.frame_out_fft.clone(),
                    formant_env: self.frame_formant.clone(),
                    color: PURPLE,
                    sample_rate: sr,
                }).width(Length::Fill).height(180),
            ].spacing(3).width(Length::FillPortion(1)),
        ].spacing(6);

        // ── Spectrograms ──────────────────────────────────────────────────────
        let make_spec_image = |img: &iced::widget::image::Handle| {
            iced::widget::image::Image::<iced::widget::image::Handle>::new(img.clone())
                .width(Length::Fill)
                .height(140u16)
                .content_fit(ContentFit::Fill)
        };

        let spectrograms = row![
            column![
                text("Input Spectrogram  (log freq, time →)").size(12),
                make_spec_image(&self.input_spec_img),
            ].spacing(3).width(Length::FillPortion(1)),
            column![
                text("Output Spectrogram  (log freq, time →)").size(12),
                make_spec_image(&self.output_spec_img),
            ].spacing(3).width(Length::FillPortion(1)),
        ].spacing(6);

        // ── Pitch tracks ──────────────────────────────────────────────────────
        let midi_freqs: Vec<f32> = if self.app_mode == AppMode::Harmony {
            if let Some(ref tl) = self.midi_timeline {
                // Show the current active MIDI target freq as a single target line
                vec![tl.freq_at(self.play_position)]
            } else { vec![] }
        } else { vec![] };

        let pitch_tracks = row![
            column![
                text("Input Pitch  (YIN — green)").size(12),
                Canvas::new(PitchTrackCanvas {
                    pitch_hz: self.input_pitch_history.iter().copied().collect(),
                    color: GREEN,
                    target_hz: vec![],
                }).width(Length::Fill).height(130),
            ].spacing(3).width(Length::FillPortion(1)),
            column![
                text("Output Pitch  (YIN — cyan)").size(12),
                Canvas::new(PitchTrackCanvas {
                    pitch_hz: self.output_pitch_history.iter().copied().collect(),
                    color: CYAN,
                    target_hz: midi_freqs,
                }).width(Length::Fill).height(130),
            ].spacing(3).width(Length::FillPortion(1)),
        ].spacing(6);

        column![
            row![
                controls,
                column![
                    transport_row,
                    panels,
                    spectrograms,
                    pitch_tracks,
                ]
                .spacing(6)
                .padding(8),
            ],
            container(text(&self.status).size(12))
                .padding([4, 16])
                .width(Length::Fill),
        ]
        .into()
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn fmt_time(samples: usize, sample_rate: u32) -> String {
    let sr = sample_rate.max(1) as usize;
    let secs = samples / sr;
    let frac = (samples % sr) * 10 / sr;
    format!("{}:{:02}.{}", secs / 60, secs % 60, frac)
}

fn bin_for_pixel(px: f32, width: f32, n_bins: usize, sample_rate: f32) -> usize {
    let log_min = 20.0_f32.log2();
    let log_max = (sample_rate / 2.0).log2();
    let freq = 2.0_f32.powf(log_min + (px / width) * (log_max - log_min));
    ((freq * 2.0 * n_bins as f32 / sample_rate) as usize).min(n_bins - 1)
}

fn freq_to_x(freq: f32, sample_rate: f32, width: f32) -> f32 {
    let log_min = 20.0_f32.log2();
    let log_max = (sample_rate / 2.0).log2();
    (freq.log2() - log_min) / (log_max - log_min) * width
}

// ── Spectrogram ───────────────────────────────────────────────────────────────

fn empty_image() -> iced::widget::image::Handle {
    iced::widget::image::Handle::from_rgba(1, 1, vec![10u8, 13, 20, 255])
}

fn build_spectrogram_image(frames: &[Vec<f32>], sample_rate: f32) -> iced::widget::image::Handle {
    const IMG_W: u32 = 1024;
    const IMG_H: u32 = 200;

    if frames.is_empty() || frames[0].is_empty() {
        return empty_image();
    }

    let n_frames = frames.len();
    let n_bins = frames[0].len();
    let global_max = frames
        .iter()
        .flat_map(|f| f.iter().copied())
        .fold(1e-10_f32, f32::max);

    let mut pixels = vec![0u8; (IMG_W * IMG_H * 4) as usize];

    for py in 0..IMG_H {
        let t = 1.0 - py as f32 / IMG_H as f32;
        let log_min = 20.0_f32.log2();
        let log_max = (sample_rate / 2.0).log2();
        let freq = 2.0_f32.powf(log_min + t * (log_max - log_min));
        let bin = ((freq * 2.0 * n_bins as f32 / sample_rate) as usize).min(n_bins - 1);

        for px in 0..IMG_W {
            let fi = (px as usize * n_frames / IMG_W as usize).min(n_frames - 1);
            let mag = frames[fi].get(bin).copied().unwrap_or(0.0);
            let db = 20.0 * (mag / global_max).max(1e-6).log10();
            let norm = ((db + 70.0) / 70.0).clamp(0.0, 1.0);
            let [r, g, b] = spectrogram_color(norm);
            let idx = ((py * IMG_W + px) * 4) as usize;
            pixels[idx] = r;
            pixels[idx + 1] = g;
            pixels[idx + 2] = b;
            pixels[idx + 3] = 255;
        }
    }
    iced::widget::image::Handle::from_rgba(IMG_W, IMG_H, pixels)
}

fn spectrogram_color(v: f32) -> [u8; 3] {
    let v = v.clamp(0.0, 1.0);
    let (r, g, b) = if v < 0.25 {
        let t = v / 0.25;
        (0.05 * t, 0.0, 0.15 * t)
    } else if v < 0.5 {
        let t = (v - 0.25) / 0.25;
        (0.05 + 0.7 * t, 0.0, 0.15 - 0.15 * t)
    } else if v < 0.75 {
        let t = (v - 0.5) / 0.25;
        (0.75 + 0.15 * t, 0.05 + 0.6 * t, 0.0)
    } else {
        let t = (v - 0.75) / 0.25;
        (0.9 + 0.1 * t, 0.65 + 0.35 * t, t)
    };
    [(r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8]
}

// ── Waveform canvas ───────────────────────────────────────────────────────────

struct WaveformView {
    samples: Vec<f32>,
    color: Color,
}

impl canvas::Program<Message> for WaveformView {
    type State = ();
    fn draw(&self, _: &(), renderer: &Renderer, _: &Theme, bounds: iced::Rectangle, _: mouse::Cursor)
        -> Vec<canvas::Geometry<Renderer>>
    {
        let mut frame = Frame::new(renderer, bounds.size());
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), PANEL);
        let cy = bounds.height / 2.0;
        let w = bounds.width;
        frame.stroke(&Path::line(Point::new(0.0, cy), Point::new(w, cy)),
            Stroke::default().with_color(GRID).with_width(1.0));
        if !self.samples.is_empty() {
            let n = self.samples.len();
            let path = Path::new(|b| {
                for px in 0..(w as usize) {
                    let idx = (px * n / w as usize).min(n - 1);
                    let y = cy - self.samples[idx].clamp(-1.0, 1.0) * cy * 0.88;
                    if px == 0 { b.move_to(Point::new(0.0, y)); } else { b.line_to(Point::new(px as f32, y)); }
                }
            });
            frame.stroke(&path, Stroke::default().with_color(self.color).with_width(1.0));
        }
        vec![frame.into_geometry()]
    }
}

// ── Spectrum canvas ───────────────────────────────────────────────────────────

struct SpectrumView {
    spectrum: Vec<f32>,
    formant_env: Vec<f32>,
    color: Color,
    sample_rate: f32,
}

impl canvas::Program<Message> for SpectrumView {
    type State = ();
    fn draw(&self, _: &(), renderer: &Renderer, _: &Theme, bounds: iced::Rectangle, _: mouse::Cursor)
        -> Vec<canvas::Geometry<Renderer>>
    {
        let mut frame = Frame::new(renderer, bounds.size());
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), PANEL);
        let w = bounds.width;
        let h = bounds.height;
        let sr = self.sample_rate;
        let n = self.spectrum.len();

        const FREQ_LABELS: &[(f32, &str)] = &[
            (100.0, "100"), (1_000.0, "1k"), (5_000.0, "5k"), (10_000.0, "10k"), (20_000.0, "20k"),
        ];
        for &(freq, label) in FREQ_LABELS {
            let x = freq_to_x(freq, sr, w);
            if x > 0.0 && x < w {
                frame.stroke(&Path::line(Point::new(x, 0.0), Point::new(x, h)),
                    Stroke::default().with_color(GRID).with_width(0.5));
                frame.fill_text(canvas::Text {
                    content: label.to_string(), position: Point::new(x, h - 14.0),
                    color: DIM, size: Pixels(10.0),
                    horizontal_alignment: alignment::Horizontal::Center,
                    ..canvas::Text::default()
                });
            }
        }

        const DB_FLOOR: f32 = -80.0;
        const DB_CEILING: f32 = 40.0;
        for db in [DB_CEILING, 0.0, -20.0, -40.0, -60.0] {
            let y = h * (1.0 - (db - DB_FLOOR) / (DB_CEILING - DB_FLOOR));
            if y >= 0.0 && y <= h {
                frame.stroke(&Path::line(Point::new(0.0, y), Point::new(w, y)),
                    Stroke::default().with_color(GRID).with_width(0.5));
                frame.fill_text(canvas::Text {
                    content: format!("{:.0}", db), position: Point::new(2.0, y - 10.0),
                    color: DIM, size: Pixels(10.0),
                    ..canvas::Text::default()
                });
            }
        }

        if self.spectrum.is_empty() { return vec![frame.into_geometry()]; }

        let c = self.color;
        let db_norm = |mag: f32| -> f32 {
            let db = 20.0 * mag.max(1e-6).log10();
            ((db - DB_FLOOR) / (DB_CEILING - DB_FLOOR)).clamp(0.0, 1.0)
        };

        let fill_path = Path::new(|b| {
            b.move_to(Point::new(0.0, h));
            for px in 0..(w as usize) {
                let bin = bin_for_pixel(px as f32, w, n, sr);
                b.line_to(Point::new(px as f32, h * (1.0 - db_norm(self.spectrum[bin]))));
            }
            b.line_to(Point::new(w, h));
            b.close();
        });
        frame.fill(&fill_path, Color { a: 0.25, ..c });

        let line_path = Path::new(|b| {
            for px in 0..(w as usize) {
                let bin = bin_for_pixel(px as f32, w, n, sr);
                let y = h * (1.0 - db_norm(self.spectrum[bin]));
                if px == 0 { b.move_to(Point::new(0.0, y)); } else { b.line_to(Point::new(px as f32, y)); }
            }
        });
        frame.stroke(&line_path, Stroke::default().with_color(c).with_width(1.5));

        if !self.formant_env.is_empty() {
            let env = Path::new(|b| {
                for px in 0..(w as usize) {
                    let bin = bin_for_pixel(px as f32, w, self.formant_env.len(), sr);
                    let y = h * (1.0 - db_norm(self.formant_env[bin]));
                    if px == 0 { b.move_to(Point::new(0.0, y)); } else { b.line_to(Point::new(px as f32, y)); }
                }
            });
            frame.stroke(&env, Stroke::default().with_color(FORMANT_COLOR).with_width(2.0));
        }
        vec![frame.into_geometry()]
    }
}

// ── Pitch track canvas ────────────────────────────────────────────────────────

struct PitchTrackCanvas {
    pitch_hz: Vec<f32>,
    color: Color,
    /// MIDI target frequencies to draw as horizontal reference lines.
    target_hz: Vec<f32>,
}

impl PitchTrackCanvas {
    fn pitch_path(series: &[f32], w: f32, pitch_y: &dyn Fn(f32) -> f32) -> Path {
        let n = series.len().max(1);
        Path::new(|b| {
            let mut in_seg = false;
            for (i, &hz) in series.iter().enumerate() {
                if hz < 50.0 { in_seg = false; continue; }
                let x = i as f32 / n as f32 * w;
                let y = pitch_y(hz);
                if !in_seg { b.move_to(Point::new(x, y)); in_seg = true; }
                else { b.line_to(Point::new(x, y)); }
            }
        })
    }
}

impl canvas::Program<Message> for PitchTrackCanvas {
    type State = ();
    fn draw(&self, _: &(), renderer: &Renderer, _: &Theme, bounds: iced::Rectangle, _: mouse::Cursor)
        -> Vec<canvas::Geometry<Renderer>>
    {
        let mut frame = Frame::new(renderer, bounds.size());
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), PANEL);
        let w = bounds.width;
        let h = bounds.height;

        let log_min = 55.0_f32.log2();
        let log_max = 1047.0_f32.log2();
        let pitch_y = |freq: f32| -> f32 {
            h * (1.0 - ((freq.log2() - log_min) / (log_max - log_min)).clamp(0.0, 1.0))
        };

        const NOTE_GRID: &[(f32, &str)] = &[
            (65.4, "C2"), (82.4, "E2"), (110.0, "A2"), (130.8, "C3"), (164.8, "E3"),
            (220.0, "A3"), (261.6, "C4"), (329.6, "E4"), (440.0, "A4"),
            (523.3, "C5"), (659.3, "E5"), (880.0, "A5"),
        ];
        for &(freq, label) in NOTE_GRID {
            let y = pitch_y(freq);
            if y > 4.0 && y < h - 4.0 {
                frame.stroke(&Path::line(Point::new(0.0, y), Point::new(w, y)),
                    Stroke::default().with_color(GRID).with_width(0.5));
                frame.fill_text(canvas::Text {
                    content: label.to_string(), position: Point::new(2.0, y - 10.0),
                    color: DIM, size: Pixels(10.0),
                    ..canvas::Text::default()
                });
            }
        }

        // MIDI target lines (dim accent)
        for &freq in &self.target_hz {
            if freq > 50.0 {
                let y = pitch_y(freq);
                if y > 0.0 && y < h {
                    frame.stroke(&Path::line(Point::new(0.0, y), Point::new(w, y)),
                        Stroke::default()
                            .with_color(Color { r: 0.35, g: 0.65, b: 1.0, a: 0.35 })
                            .with_width(2.0));
                }
            }
        }

        // Pitch line
        if !self.pitch_hz.is_empty() {
            let path = Self::pitch_path(&self.pitch_hz, w, &pitch_y);
            frame.stroke(&path, Stroke::default().with_color(self.color).with_width(2.0));
        }

        vec![frame.into_geometry()]
    }
}
