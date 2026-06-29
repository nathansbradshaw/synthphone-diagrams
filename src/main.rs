use std::collections::VecDeque;
use std::fmt;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

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
mod realtime;

use audio::{AudioData, DspParams};
use realtime::{HALF, N, VisualizationSnapshot};

// ── Rodio output stream (thread-local — OutputStream is !Send) ────────────────
thread_local! {
    static AUDIO_OUT: std::cell::RefCell<Option<(
        rodio::OutputStream,
        rodio::OutputStreamHandle,
    )>> = const { std::cell::RefCell::new(None) };
}

// ── Colour palette ────────────────────────────────────────────────────────────
const PANEL: Color = Color {
    r: 0.08,
    g: 0.10,
    b: 0.14,
    a: 1.0,
};
const GRID: Color = Color {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 0.06,
};
const DIM: Color = Color {
    r: 0.45,
    g: 0.50,
    b: 0.55,
    a: 1.0,
};
const GREEN: Color = Color {
    r: 0.27,
    g: 1.0,
    b: 0.53,
    a: 1.0,
};
const CYAN: Color = Color {
    r: 0.27,
    g: 0.87,
    b: 1.0,
    a: 1.0,
};
const ORANGE: Color = Color {
    r: 1.0,
    g: 0.53,
    b: 0.27,
    a: 1.0,
};
const PURPLE: Color = Color {
    r: 0.80,
    g: 0.27,
    b: 1.0,
    a: 1.0,
};
const FORMANT_COLOR: Color = Color {
    r: 0.3,
    g: 1.0,
    b: 0.4,
    a: 0.9,
};

// ── Mode / preset enums ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppMode {
    PitchCorrection,
    Vocode,
    Dry,
}

impl fmt::Display for AppMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppMode::PitchCorrection => write!(f, "Pitch Correction"),
            AppMode::Vocode => write!(f, "Vocoder"),
            AppMode::Dry => write!(f, "Dry / Formant"),
        }
    }
}

const APP_MODES: [AppMode; 3] = [AppMode::PitchCorrection, AppMode::Vocode, AppMode::Dry];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CarrierWave {
    Sine,
    Saw,
    Square,
    Triangle,
}

impl fmt::Display for CarrierWave {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            CarrierWave::Sine => "Sine",
            CarrierWave::Saw => "Saw",
            CarrierWave::Square => "Square",
            CarrierWave::Triangle => "Triangle",
        };
        write!(f, "{s}")
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
    CarrierWave::Sine,
    CarrierWave::Saw,
    CarrierWave::Square,
    CarrierWave::Triangle,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FormantMode {
    Off,
    Male,
    Female,
}

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
        .run_with(App::new)
}

// ── Application state ─────────────────────────────────────────────────────────

const MAX_SPEC_COLS: usize = 512;
const MAX_PITCH_HISTORY: usize = 512;

struct App {
    audio: Option<AudioData>,
    status: String,

    // Real-time shared state (written by DSP thread, read by GUI)
    viz: Arc<Mutex<VisualizationSnapshot>>,
    dsp_params: Arc<Mutex<DspParams>>,
    dsp_stop: Arc<AtomicBool>,
    dsp_thread: Option<std::thread::JoinHandle<()>>,
    rodio_sink: Option<rodio::Sink>,

    // Current frame displayed in canvases (snapshotted on VizTick)
    frame_input: Vec<f32>,
    frame_output: Vec<f32>,
    frame_in_fft: Vec<f32>,
    frame_out_fft: Vec<f32>,
    frame_formant: Vec<f32>,
    frame_pitch_hz: f32,

    // Rolling spectrogram (VecDeque of columns, rendered as image each tick)
    spectrogram_cols: VecDeque<Vec<f32>>,
    last_col_serial: u64,
    spectrogram_img: iced::widget::image::Handle,

    // Scrolling pitch track history
    pitch_history: VecDeque<f32>,

    // Playback position for transport bar
    play_position: usize,
    total_samples: usize,
    is_playing: bool,

    // UI parameter state
    app_mode: AppMode,
    config: VocalEffectsConfig, // holds sample_rate, hop_size
    pitch_strength: f32,
    octave: f32,
    carrier_wave: CarrierWave,
    carrier_freq: f32,
    formant_mode: FormantMode,
    formant_ratio: f32,
    bit_depth: f32,
    sample_rate_div: f32,
}

#[derive(Debug, Clone)]
enum Message {
    OpenFile,
    FileOpened(Option<std::path::PathBuf>),
    VizTick,
    // Transport
    PlayPause,
    Stop,
    Seek(f32),
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
        }));
        (
            Self {
                audio: None,
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
                spectrogram_cols: VecDeque::new(),
                last_col_serial: 0,
                spectrogram_img: empty_image(),
                pitch_history: VecDeque::new(),
                play_position: 0,
                total_samples: 0,
                is_playing: false,
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
            },
            Task::none(),
        )
    }

    fn subscription(&self) -> Subscription<Message> {
        if self.audio.is_some() {
            iced::time::every(std::time::Duration::from_millis(33)).map(|_| Message::VizTick)
        } else {
            Subscription::none()
        }
    }

    // ── Build DspParams from current UI state ─────────────────────────────────

    fn build_dsp_params(&self) -> DspParams {
        let mut config = self.config.clone();
        let mut settings = MusicalSettings::default();

        config.pitch_correction_strength = self.pitch_strength;
        settings.octave = self.octave.round() as i8;

        // Voice mode
        settings.mode = match self.app_mode {
            AppMode::PitchCorrection => ProcessingMode::PitchControl,
            AppMode::Vocode => ProcessingMode::Vocode,
            AppMode::Dry => ProcessingMode::Dry,
        };

        // Formant applies to all modes (like the SynthPhone effects layer)
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
            // Bitcrush / SR reduction apply to all modes
            bit_depth: self.bit_depth.round() as i8,
            sample_rate_div: self.sample_rate_div.round() as i8,
            carrier_freq: self.carrier_freq,
            carrier_waveform: self.carrier_wave.into(),
        }
    }

    // Push updated params to the running DSP thread (takes effect next frame).
    fn push_params(&self) {
        *self.dsp_params.lock().unwrap() = self.build_dsp_params();
    }

    // ── Playback control ──────────────────────────────────────────────────────

    fn stop_dsp(&mut self) {
        self.dsp_stop.store(true, Ordering::Relaxed);
        // Stop the sink FIRST — this drops ChannelSource (and its rx),
        // which unblocks any DSP thread send() so the join won't deadlock.
        if let Some(s) = self.rodio_sink.take() {
            s.stop();
        }
        if let Some(t) = self.dsp_thread.take() {
            let _ = t.join();
        }
        self.dsp_stop = Arc::new(AtomicBool::new(false));
        self.is_playing = false;
    }

    fn start_dsp(&mut self, from_sample: usize) {
        // Clone what we need before mutably borrowing self via stop_dsp().
        let (samples, sample_rate) = match self.audio {
            Some(ref a) => (a.samples.clone(), a.sample_rate),
            None => return,
        };

        self.stop_dsp();
        self.push_params();

        // Reset viz so a stale play_position doesn't trigger end-of-file
        // detection before the new DSP thread produces its first frame.
        {
            let mut v = self.viz.lock().unwrap();
            v.play_position = from_sample;
            v.col_serial = 0;
        }
        self.last_col_serial = 0;

        // Initialise rodio output stream lazily.
        AUDIO_OUT.with(|cell| {
            let mut opt = cell.borrow_mut();
            if opt.is_none() {
                match rodio::OutputStream::try_default() {
                    Ok((s, h)) => *opt = Some((s, h)),
                    Err(e) => self.status = format!("Audio init failed: {e}"),
                }
            }
        });

        let sink = AUDIO_OUT.with(|cell| {
            cell.borrow()
                .as_ref()
                .and_then(|(_, h)| rodio::Sink::try_new(h).ok())
        });

        let Some(sink) = sink else { return };

        let (handle, source) = realtime::start_dsp_thread(
            samples,
            sample_rate,
            from_sample,
            Arc::clone(&self.dsp_params),
            Arc::clone(&self.viz),
            Arc::clone(&self.dsp_stop),
        );

        sink.append(source);
        sink.play();

        self.rodio_sink = Some(sink);
        self.dsp_thread = Some(handle);
        self.is_playing = true;
    }

    // ── Update ────────────────────────────────────────────────────────────────

    fn update(&mut self, msg: Message) -> Task<Message> {
        match msg {
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
                    self.spectrogram_cols = VecDeque::new();
                    self.pitch_history = VecDeque::new();
                    self.last_col_serial = 0;
                    self.audio = Some(data);
                }
                Err(e) => self.status = format!("Error loading file: {e}"),
            },
            Message::FileOpened(None) => {}

            // ── Transport ─────────────────────────────────────────────────────
            Message::PlayPause => {
                if self.is_playing {
                    self.stop_dsp();
                    // Freeze at current viz position.
                    self.play_position = self.viz.lock().unwrap().play_position;
                } else {
                    self.start_dsp(self.play_position);
                }
            }
            Message::Stop => {
                self.stop_dsp();
                self.play_position = 0;
                self.spectrogram_cols = VecDeque::new();
                self.pitch_history = VecDeque::new();
            }
            Message::Seek(frac) => {
                let new_pos = (frac * self.total_samples as f32) as usize;
                self.play_position = new_pos;
                self.spectrogram_cols = VecDeque::new();
                self.pitch_history = VecDeque::new();
                if self.is_playing {
                    self.start_dsp(new_pos);
                }
            }

            // ── Real-time visualization tick ──────────────────────────────────
            Message::VizTick => {
                let (col_serial, play_pos, total, pitch_hz) = {
                    let v = self.viz.lock().unwrap();
                    (v.col_serial, v.play_position, v.total_samples, v.pitch_hz)
                };

                // Append new spectrogram columns since last tick.
                if col_serial > self.last_col_serial {
                    let v = self.viz.lock().unwrap();
                    self.spectrogram_cols.push_back(v.spectrogram_col.clone());
                    if self.spectrogram_cols.len() > MAX_SPEC_COLS {
                        self.spectrogram_cols.pop_front();
                    }
                    self.last_col_serial = col_serial;

                    // Snapshot current frame data for canvas rendering.
                    self.frame_input = v.input_window.clone();
                    self.frame_output = v.output_window.clone();
                    self.frame_in_fft = v.input_fft.clone();
                    self.frame_out_fft = v.output_fft.clone();
                    self.frame_formant = v.formant_envelope.clone();
                    self.frame_pitch_hz = v.pitch_hz;
                }

                self.play_position = play_pos;
                self.total_samples = total.max(self.total_samples);

                // Append to scrolling pitch history.
                self.pitch_history.push_back(pitch_hz);
                if self.pitch_history.len() > MAX_PITCH_HISTORY {
                    self.pitch_history.pop_front();
                }

                // End of file → loop from the beginning.
                if self.is_playing && total > 0 && play_pos >= total {
                    self.spectrogram_cols.clear();
                    self.pitch_history.clear();
                    self.start_dsp(0);
                }

                // Rebuild spectrogram image every tick.
                let cols: Vec<Vec<f32>> = self.spectrogram_cols.iter().cloned().collect();
                self.spectrogram_img = build_spectrogram_image(&cols, self.config.sample_rate);
            }

            // ── Parameter changes: push to running DSP thread immediately ─────
            Message::ModeChanged(m) => {
                self.app_mode = m;
                self.push_params();
            }
            Message::PitchStrengthChanged(v) => {
                self.pitch_strength = v;
                self.push_params();
            }
            Message::OctaveChanged(v) => {
                self.octave = v;
                self.push_params();
            }
            Message::CarrierWaveChanged(w) => {
                self.carrier_wave = w;
                self.push_params();
            }
            Message::CarrierFreqChanged(v) => {
                self.carrier_freq = v;
                self.push_params();
            }
            Message::FormantModeChanged(m) => {
                self.formant_mode = m;
                self.push_params();
            }
            Message::FormantRatioChanged(v) => {
                self.formant_ratio = v;
                self.push_params();
            }
            Message::BitDepthChanged(v) => {
                self.bit_depth = v;
                self.push_params();
            }
            Message::SampleRateDivChanged(v) => {
                self.sample_rate_div = v;
                self.push_params();
            }
        }
        Task::none()
    }

    // ── View ──────────────────────────────────────────────────────────────────

    fn view(&self) -> Element<'_, Message> {
        let sr = self.config.sample_rate;
        let play_frac = if self.total_samples > 0 {
            self.play_position as f32 / self.total_samples as f32
        } else {
            0.0
        };

        // ── Mode picker ───────────────────────────────────────────────────────
        let mode_row = row![
            text("Mode").size(13),
            pick_list(&APP_MODES[..], Some(self.app_mode), Message::ModeChanged),
        ]
        .spacing(8);

        // ── Mode-specific controls ────────────────────────────────────────────
        let mode_controls: Element<'_, Message> = match self.app_mode {
            AppMode::PitchCorrection => column![
                text("Correction Strength").size(13),
                slider(
                    0.0..=1.0_f32,
                    self.pitch_strength,
                    Message::PitchStrengthChanged
                )
                .step(0.01_f32),
                text(format!("{:.2}", self.pitch_strength)).size(12),
                text("Pitch Shift").size(13),
                slider(1.0_f32..=8.0_f32, self.octave, Message::OctaveChanged).step(0.1_f32),
                text(format!("{:.1}× ({:.1}×)", self.octave, self.octave * 0.5)).size(12),
            ]
            .spacing(6)
            .into(),

            AppMode::Vocode => column![
                text("Carrier Waveform").size(13),
                pick_list(
                    &CARRIER_WAVES[..],
                    Some(self.carrier_wave),
                    Message::CarrierWaveChanged
                ),
                text("Carrier Freq (Hz)").size(13),
                slider(
                    50.0_f32..=2000.0_f32,
                    self.carrier_freq,
                    Message::CarrierFreqChanged
                )
                .step(1.0_f32),
                text(format!("{:.0} Hz", self.carrier_freq)).size(12),
            ]
            .spacing(6)
            .into(),

            AppMode::Dry => column![
                text("Pitch Shift").size(13),
                slider(1.0_f32..=8.0_f32, self.octave, Message::OctaveChanged).step(0.1_f32),
                text(format!("{:.1}× ({:.1}×)", self.octave, self.octave * 0.5)).size(12),
            ]
            .spacing(6)
            .into(),
        };

        // ── Global effects (apply to every voice mode) ────────────────────────
        let effects_controls = column![
            text("── Effects ──").size(12),
            text("Formant").size(13),
            pick_list(
                &FORMANT_MODES[..],
                Some(self.formant_mode),
                Message::FormantModeChanged
            ),
            slider(
                0.25_f32..=4.0_f32,
                self.formant_ratio,
                Message::FormantRatioChanged
            )
            .step(0.05_f32),
            text(format!("Ratio {:.2}×", self.formant_ratio)).size(12),
            text("Bit Depth").size(13),
            slider(1.0_f32..=16.0_f32, self.bit_depth, Message::BitDepthChanged).step(1.0_f32),
            text(format!("{} bit", self.bit_depth as i8)).size(12),
            text("Sample Rate ÷").size(13),
            slider(
                1.0_f32..=32.0_f32,
                self.sample_rate_div,
                Message::SampleRateDivChanged
            )
            .step(1.0_f32),
            text(format!("÷{}", self.sample_rate_div as i8)).size(12),
        ]
        .spacing(4);

        let controls = column![
            button("Open WAV file").on_press(Message::OpenFile),
            mode_row,
            mode_controls,
            effects_controls,
        ]
        .spacing(12)
        .padding(16)
        .width(230);

        // ── Transport ─────────────────────────────────────────────────────────
        let has_audio = self.audio.is_some();
        let play_label = if self.is_playing { "Pause" } else { "Play" };

        let transport = row![
            if has_audio {
                button(play_label).on_press(Message::PlayPause)
            } else {
                button(play_label)
            },
            if has_audio {
                button("Stop").on_press(Message::Stop)
            } else {
                button("Stop")
            },
            slider(0.0_f32..=1.0_f32, play_frac, Message::Seek)
                .step(0.0005_f32)
                .width(Length::Fill),
            text(format!(
                "{} / {}",
                fmt_time(self.play_position, sr as u32),
                fmt_time(self.total_samples, sr as u32),
            ))
            .size(12),
        ]
        .spacing(8)
        .padding([4, 0]);

        let out_label = format!("Output  ({})", self.app_mode);

        // ── Waveform + spectrum panels ────────────────────────────────────────
        let panels = row![
            column![
                text("Input  (current block)").size(12),
                Canvas::new(WaveformView {
                    samples: self.frame_input.clone(),
                    color: GREEN
                })
                .width(Length::Fill)
                .height(90),
                text("Input · Spectrum + formant envelope").size(12),
                Canvas::new(SpectrumView {
                    spectrum: self.frame_in_fft.clone(),
                    formant_env: self.frame_formant.clone(),
                    color: ORANGE,
                    sample_rate: sr,
                })
                .width(Length::Fill)
                .height(130),
            ]
            .spacing(3)
            .width(Length::FillPortion(1)),
            column![
                text(out_label).size(12),
                Canvas::new(WaveformView {
                    samples: self.frame_output.clone(),
                    color: CYAN
                })
                .width(Length::Fill)
                .height(90),
                text("Output · Spectrum  + formant envelope").size(12),
                Canvas::new(SpectrumView {
                    spectrum: self.frame_out_fft.clone(),
                    formant_env: self.frame_formant.clone(),
                    color: PURPLE,
                    sample_rate: sr,
                })
                .width(Length::Fill)
                .height(130),
            ]
            .spacing(3)
            .width(Length::FillPortion(1)),
        ]
        .spacing(6);

        // ── Spectrogram ───────────────────────────────────────────────────────
        let spectrogram = iced::widget::image::Image::<iced::widget::image::Handle>::new(
            self.spectrogram_img.clone(),
        )
        .width(Length::Fill)
        .height(160u16)
        .content_fit(ContentFit::Fill);

        // ── Pitch track ───────────────────────────────────────────────────────
        let pitch_panel = Canvas::new(PitchTrackView {
            pitch_track_hz: self.pitch_history.iter().copied().collect(),
            sample_rate: sr,
        })
        .width(Length::Fill)
        .height(140);

        column![
            row![
                controls,
                column![
                    transport,
                    panels,
                    text("Spectrogram  (log freq, time →, scrolling)").size(12),
                    spectrogram,
                    text("Pitch track  (YIN — last ~17s)").size(12),
                    pitch_panel,
                ]
                .spacing(3)
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

fn freq_to_y(freq: f32, sample_rate: f32, height: f32) -> f32 {
    let log_min = 20.0_f32.log2();
    let log_max = (sample_rate / 2.0).log2();
    height * (1.0 - ((freq.log2() - log_min) / (log_max - log_min)).clamp(0.0, 1.0))
}

// ── Spectrogram image builder ─────────────────────────────────────────────────

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
    fn draw(
        &self,
        _: &(),
        renderer: &Renderer,
        _: &Theme,
        bounds: iced::Rectangle,
        _: mouse::Cursor,
    ) -> Vec<canvas::Geometry<Renderer>> {
        let mut frame = Frame::new(renderer, bounds.size());
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), PANEL);

        let cy = bounds.height / 2.0;
        let w = bounds.width;
        frame.stroke(
            &Path::line(Point::new(0.0, cy), Point::new(w, cy)),
            Stroke::default().with_color(GRID).with_width(1.0),
        );

        if !self.samples.is_empty() {
            let n = self.samples.len();
            let path = Path::new(|b| {
                for px in 0..(w as usize) {
                    let idx = (px * n / w as usize).min(n - 1);
                    let y = cy - self.samples[idx].clamp(-1.0, 1.0) * cy * 0.88;
                    if px == 0 {
                        b.move_to(Point::new(0.0, y));
                    } else {
                        b.line_to(Point::new(px as f32, y));
                    }
                }
            });
            frame.stroke(
                &path,
                Stroke::default().with_color(self.color).with_width(1.0),
            );
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
    fn draw(
        &self,
        _: &(),
        renderer: &Renderer,
        _: &Theme,
        bounds: iced::Rectangle,
        _: mouse::Cursor,
    ) -> Vec<canvas::Geometry<Renderer>> {
        let mut frame = Frame::new(renderer, bounds.size());
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), PANEL);

        let w = bounds.width;
        let h = bounds.height;
        let sr = self.sample_rate;
        let n = self.spectrum.len();

        const FREQ_LABELS: &[(f32, &str)] = &[
            (100.0, "100"),
            (1_000.0, "1k"),
            (5_000.0, "5k"),
            (10_000.0, "10k"),
            (20_000.0, "20k"),
        ];

        for &(freq, label) in FREQ_LABELS {
            let x = freq_to_x(freq, sr, w);
            if x > 0.0 && x < w {
                frame.stroke(
                    &Path::line(Point::new(x, 0.0), Point::new(x, h)),
                    Stroke::default().with_color(GRID).with_width(0.5),
                );
                frame.fill_text(canvas::Text {
                    content: label.to_string(),
                    position: Point::new(x, h - 14.0),
                    color: DIM,
                    size: Pixels(10.0),
                    horizontal_alignment: alignment::Horizontal::Center,
                    ..canvas::Text::default()
                });
            }
        }

        const DB_FLOOR: f32 = -80.0;
        for db in [-20.0_f32, -40.0, -60.0] {
            let y = h * (1.0 - (db - DB_FLOOR) / (-DB_FLOOR));
            frame.stroke(
                &Path::line(Point::new(0.0, y), Point::new(w, y)),
                Stroke::default().with_color(GRID).with_width(0.5),
            );
            frame.fill_text(canvas::Text {
                content: format!("{db}"),
                position: Point::new(2.0, y - 10.0),
                color: DIM,
                size: Pixels(10.0),
                ..canvas::Text::default()
            });
        }

        if self.spectrum.is_empty() {
            return vec![frame.into_geometry()];
        }

        let c = self.color;
        let db_norm = |mag: f32| -> f32 {
            let db = 20.0 * mag.max(1e-6).log10();
            ((db - DB_FLOOR) / (-DB_FLOOR)).clamp(0.0, 1.0)
        };

        let fill_path = Path::new(|b| {
            b.move_to(Point::new(0.0, h));
            for px in 0..(w as usize) {
                let bin = bin_for_pixel(px as f32, w, n, sr);
                b.line_to(Point::new(
                    px as f32,
                    h * (1.0 - db_norm(self.spectrum[bin])),
                ));
            }
            b.line_to(Point::new(w, h));
            b.close();
        });
        frame.fill(&fill_path, Color { a: 0.25, ..c });

        let line_path = Path::new(|b| {
            for px in 0..(w as usize) {
                let bin = bin_for_pixel(px as f32, w, n, sr);
                let y = h * (1.0 - db_norm(self.spectrum[bin]));
                if px == 0 {
                    b.move_to(Point::new(0.0, y));
                } else {
                    b.line_to(Point::new(px as f32, y));
                }
            }
        });
        frame.stroke(&line_path, Stroke::default().with_color(c).with_width(1.5));

        if !self.formant_env.is_empty() {
            let env = Path::new(|b| {
                for px in 0..(w as usize) {
                    let bin = bin_for_pixel(px as f32, w, self.formant_env.len(), sr);
                    let y = h * (1.0 - db_norm(self.formant_env[bin]));
                    if px == 0 {
                        b.move_to(Point::new(0.0, y));
                    } else {
                        b.line_to(Point::new(px as f32, y));
                    }
                }
            });
            frame.stroke(
                &env,
                Stroke::default().with_color(FORMANT_COLOR).with_width(2.0),
            );
        }
        vec![frame.into_geometry()]
    }
}

// ── Pitch track canvas ────────────────────────────────────────────────────────

struct PitchTrackView {
    pitch_track_hz: Vec<f32>,
    sample_rate: f32,
}

impl canvas::Program<Message> for PitchTrackView {
    type State = ();
    fn draw(
        &self,
        _: &(),
        renderer: &Renderer,
        _: &Theme,
        bounds: iced::Rectangle,
        _: mouse::Cursor,
    ) -> Vec<canvas::Geometry<Renderer>> {
        let mut frame = Frame::new(renderer, bounds.size());
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), PANEL);

        let w = bounds.width;
        let h = bounds.height;

        // Vocal pitch range: ~55 Hz (A1) to ~1047 Hz (C6) — log scale.
        let log_min = 55.0_f32.log2();
        let log_max = 1047.0_f32.log2();
        let pitch_y = |freq: f32| -> f32 {
            h * (1.0 - ((freq.log2() - log_min) / (log_max - log_min)).clamp(0.0, 1.0))
        };

        const NOTE_GRID: &[(f32, &str)] = &[
            (65.4, "C2"),
            (82.4, "E2"),
            (110.0, "A2"),
            (130.8, "C3"),
            (164.8, "E3"),
            (220.0, "A3"),
            (261.6, "C4"),
            (329.6, "E4"),
            (440.0, "A4"),
            (523.3, "C5"),
            (659.3, "E5"),
            (880.0, "A5"),
        ];
        for &(freq, label) in NOTE_GRID {
            let y = pitch_y(freq);
            if y > 4.0 && y < h - 4.0 {
                frame.stroke(
                    &Path::line(Point::new(0.0, y), Point::new(w, y)),
                    Stroke::default().with_color(GRID).with_width(0.5),
                );
                frame.fill_text(canvas::Text {
                    content: label.to_string(),
                    position: Point::new(2.0, y - 10.0),
                    color: DIM,
                    size: Pixels(10.0),
                    ..canvas::Text::default()
                });
            }
        }

        if self.pitch_track_hz.is_empty() {
            return vec![frame.into_geometry()];
        }

        let n = self.pitch_track_hz.len();
        let path = Path::new(|b| {
            let mut in_seg = false;
            for (i, &hz) in self.pitch_track_hz.iter().enumerate() {
                if hz < 50.0 {
                    in_seg = false;
                    continue;
                }
                let x = i as f32 / n as f32 * w;
                let y = pitch_y(hz);
                if !in_seg {
                    b.move_to(Point::new(x, y));
                    in_seg = true;
                } else {
                    b.line_to(Point::new(x, y));
                }
            }
        });
        frame.stroke(&path, Stroke::default().with_color(GREEN).with_width(2.0));

        vec![frame.into_geometry()]
    }
}
