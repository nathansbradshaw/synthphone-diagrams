use std::fmt;
use std::time::Instant;

use iced::widget::canvas::{self, Frame, Path, Stroke};
use iced::widget::{button, column, container, pick_list, row, slider, text, Canvas};
use iced::{
    alignment, Color, ContentFit, Element, Length, Pixels, Point, Renderer,
    Subscription, Task, Theme,
};
use iced::mouse;

use synthphone_e_vocal_dsp::{MusicalSettings, ProcessingMode, VocalEffectsConfig};
use synthphone_e_vocal_dsp::audio::Waveform;

mod audio;
use audio::{AudioData, DspParams, ProcessedAudio};

// ── Rodio output stream (thread-local — OutputStream is !Send) ────────────────
thread_local! {
    static AUDIO_OUT: std::cell::RefCell<Option<(
        rodio::OutputStream,
        rodio::OutputStreamHandle,
    )>> = const { std::cell::RefCell::new(None) };
}

// ── Colour palette ────────────────────────────────────────────────────────────
const PANEL: Color = Color { r: 0.08, g: 0.10, b: 0.14, a: 1.0 };
const GRID:  Color = Color { r: 1.0,  g: 1.0,  b: 1.0,  a: 0.06 };
const DIM:   Color = Color { r: 0.45, g: 0.50, b: 0.55, a: 1.0 };
const GREEN:  Color = Color { r: 0.27, g: 1.0,  b: 0.53, a: 1.0 };
const CYAN:   Color = Color { r: 0.27, g: 0.87, b: 1.0,  a: 1.0 };
const ORANGE: Color = Color { r: 1.0,  g: 0.53, b: 0.27, a: 1.0 };
const PURPLE: Color = Color { r: 0.80, g: 0.27, b: 1.0,  a: 1.0 };
const FORMANT_COLOR: Color = Color { r: 0.3, g: 1.0, b: 0.4, a: 0.9 };

// ── App mode ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppMode { PitchCorrection, Vocode, FormantShift, Bitcrusher }

impl fmt::Display for AppMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppMode::PitchCorrection => write!(f, "Pitch Correction"),
            AppMode::Vocode          => write!(f, "Vocoder"),
            AppMode::FormantShift    => write!(f, "Formant Shift"),
            AppMode::Bitcrusher      => write!(f, "Bitcrusher"),
        }
    }
}

const APP_MODES: [AppMode; 4] = [
    AppMode::PitchCorrection,
    AppMode::Vocode,
    AppMode::FormantShift,
    AppMode::Bitcrusher,
];

// ── Carrier waveform ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CarrierWave { Sine, Saw, Square, Triangle }

impl fmt::Display for CarrierWave {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CarrierWave::Sine     => write!(f, "Sine"),
            CarrierWave::Saw      => write!(f, "Saw"),
            CarrierWave::Square   => write!(f, "Square"),
            CarrierWave::Triangle => write!(f, "Triangle"),
        }
    }
}

impl From<CarrierWave> for Waveform {
    fn from(cw: CarrierWave) -> Waveform {
        match cw {
            CarrierWave::Sine     => Waveform::Sine,
            CarrierWave::Saw      => Waveform::Saw,
            CarrierWave::Square   => Waveform::Square,
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

// ── Formant preset ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FormantMode { Off, Male, Female }

impl fmt::Display for FormantMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FormantMode::Off    => write!(f, "Off"),
            FormantMode::Male   => write!(f, "Male (↓)"),
            FormantMode::Female => write!(f, "Female (↑)"),
        }
    }
}

const FORMANT_MODES: [FormantMode; 3] =
    [FormantMode::Off, FormantMode::Male, FormantMode::Female];

// ── Entry point ───────────────────────────────────────────────────────────────

fn main() -> iced::Result {
    iced::application("SynthPhone DSP Viewer", App::update, App::view)
        .theme(|_| Theme::Dark)
        .subscription(App::subscription)
        .run_with(App::new)
}

// ── Application state ─────────────────────────────────────────────────────────

struct App {
    audio:               Option<AudioData>,
    processed:           Option<ProcessedAudio>,
    spectrogram_handle:  iced::widget::image::Handle,
    config:              VocalEffectsConfig,
    settings:            MusicalSettings,
    file_status:         String,   // "filename · rate · samples"
    status:              String,   // current display (may say "Processing…")
    // Mode
    app_mode:            AppMode,
    // Pitch correction
    pitch_strength:      f32,
    octave:              f32,
    // Vocoder
    carrier_wave:        CarrierWave,
    carrier_freq:        f32,
    // Formant shift
    formant_mode:        FormantMode,
    formant_ratio:       f32,
    // Bitcrusher
    bit_depth:           f32,
    sample_rate_div:     f32,
    // Playback
    sink:                Option<rodio::Sink>,
    play_start:          Option<(Instant, usize)>,
    is_playing:          bool,
    play_position:       usize,
    // Background processing
    processing_gen:      u64,     // incremented on each reprocess call
}

#[derive(Debug, Clone)]
enum Message {
    OpenFile,
    FileOpened(Option<std::path::PathBuf>),
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
    // Transport
    PlayPause,
    Stop,
    Seek(f32),
    PlayTick,
    // Background DSP result: (generation, processed audio, pre-built spectrogram)
    ProcessingDone(u64, Box<ProcessedAudio>, iced::widget::image::Handle),
}

impl App {
    fn new() -> (Self, Task<Message>) {
        let config   = VocalEffectsConfig::default();
        let settings = MusicalSettings::default();
        (
            Self {
                audio:              None,
                processed:          None,
                spectrogram_handle: empty_image(),
                config,
                settings,
                file_status:        String::new(),
                status:             "Open a WAV file to begin.".into(),
                app_mode:           AppMode::PitchCorrection,
                pitch_strength:     0.8,
                octave:             0.0,
                carrier_wave:       CarrierWave::Saw,
                carrier_freq:       110.0,
                formant_mode:       FormantMode::Off,
                formant_ratio:      1.0,
                bit_depth:          16.0,
                sample_rate_div:    1.0,
                sink:               None,
                play_start:         None,
                is_playing:         false,
                play_position:      0,
                processing_gen:     0,
            },
            Task::none(),
        )
    }

    fn subscription(&self) -> Subscription<Message> {
        if self.is_playing {
            iced::time::every(std::time::Duration::from_millis(40))
                .map(|_| Message::PlayTick)
        } else {
            Subscription::none()
        }
    }

    fn build_dsp_params(&self) -> DspParams {
        let mut config   = self.config.clone();
        let mut settings = self.settings;

        config.pitch_correction_strength = self.pitch_strength;
        settings.octave = self.octave.round() as i8;

        match self.app_mode {
            AppMode::PitchCorrection => {
                settings.mode    = ProcessingMode::PitchControl;
                settings.formant = 0;
            }
            AppMode::Vocode => {
                settings.mode    = ProcessingMode::Vocode;
                settings.formant = 0;
            }
            AppMode::FormantShift => {
                settings.mode    = ProcessingMode::Dry;
                settings.formant = match self.formant_mode {
                    FormantMode::Off    => 0,
                    FormantMode::Male   => 1,
                    FormantMode::Female => 2,
                };
                settings.formant_male_ratio   = self.formant_ratio.min(1.0);
                settings.formant_female_ratio = self.formant_ratio.max(1.0);
            }
            AppMode::Bitcrusher => {
                settings.mode    = ProcessingMode::Dry;
                settings.formant = 0;
            }
        }

        DspParams {
            config,
            settings,
            bit_depth: if self.app_mode == AppMode::Bitcrusher {
                self.bit_depth.round() as i8
            } else {
                16
            },
            sample_rate_div: if self.app_mode == AppMode::Bitcrusher {
                self.sample_rate_div.round() as i8
            } else {
                1
            },
            carrier_freq:     self.carrier_freq,
            carrier_waveform: self.carrier_wave.into(),
        }
    }

    // Spawns background processing; returns the Task to propagate to iced.
    fn reprocess(&mut self) -> Task<Message> {
        // Clone samples first to release the immutable borrow before any mutation.
        let Some(samples) = self.audio.as_ref().map(|a| a.samples.clone()) else {
            return Task::none();
        };

        self.stop_playback();
        self.play_position = 0;

        self.processing_gen += 1;
        let proc_gen    = self.processing_gen;
        let params      = self.build_dsp_params();
        let sample_rate = self.config.sample_rate;

        self.status = format!("Processing…  ({})", self.app_mode);

        Task::perform(
            // Run the heavy work on a blocking thread so the GUI stays live.
            async move {
                tokio::task::spawn_blocking(move || -> (u64, ProcessedAudio, iced::widget::image::Handle) {
                    let processed = audio::process_samples(&samples, &params);
                    let handle    = build_spectrogram_image(
                        &processed.spectrogram_frames,
                        sample_rate,
                    );
                    (proc_gen, processed, handle)
                })
                .await
                .expect("DSP thread panicked")
            },
            |(proc_gen, processed, handle)| {
                Message::ProcessingDone(proc_gen, Box::new(processed), handle)
            },
        )
    }

    // ── Playback helpers ──────────────────────────────────────────────────────

    fn stop_playback(&mut self) {
        if let Some(sink) = self.sink.take() { sink.stop(); }
        self.is_playing = false;
        self.play_start = None;
    }

    fn start_playback(&mut self) {
        let Some(ref processed) = self.processed else { return };
        let sr = self.audio.as_ref().map(|a| a.sample_rate).unwrap_or(44_100);

        if let Some(sink) = self.sink.take() { sink.stop(); }

        AUDIO_OUT.with(|cell| {
            let mut opt = cell.borrow_mut();
            if opt.is_none() {
                match rodio::OutputStream::try_default() {
                    Ok((stream, handle)) => *opt = Some((stream, handle)),
                    Err(e) => self.status = format!("Audio init failed: {e}"),
                }
            }
        });

        let new_sink = AUDIO_OUT.with(|cell| {
            cell.borrow()
                .as_ref()
                .and_then(|(_, handle)| rodio::Sink::try_new(handle).ok())
        });

        if let Some(sink) = new_sink {
            let pos     = self.play_position.min(processed.samples.len().saturating_sub(1));
            let samples = processed.samples[pos..].to_vec();
            let source  = rodio::buffer::SamplesBuffer::new(1, sr, samples);
            sink.append(source);
            sink.play();
            self.sink       = Some(sink);
            self.play_start = Some((Instant::now(), pos));
            self.is_playing = true;
        }
    }

    // ── Update ────────────────────────────────────────────────────────────────

    fn update(&mut self, msg: Message) -> Task<Message> {
        match msg {
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
                    self.file_status = format!(
                        "{}  ·  {}Hz  ·  {} samples",
                        path.file_name().unwrap_or_default().to_string_lossy(),
                        data.sample_rate,
                        data.samples.len(),
                    );
                    self.audio         = Some(data);
                    self.play_position = 0;
                    return self.reprocess();
                }
                Err(e) => self.status = format!("Error loading file: {e}"),
            },
            Message::FileOpened(None) => {}

            // All parameter changes just kick off a background reprocess.
            Message::ModeChanged(m)          => { self.app_mode        = m; return self.reprocess(); }
            Message::PitchStrengthChanged(v) => { self.pitch_strength  = v; return self.reprocess(); }
            Message::OctaveChanged(v)        => { self.octave          = v; return self.reprocess(); }
            Message::CarrierWaveChanged(w)   => { self.carrier_wave    = w; return self.reprocess(); }
            Message::CarrierFreqChanged(v)   => { self.carrier_freq    = v; return self.reprocess(); }
            Message::FormantModeChanged(m)   => { self.formant_mode    = m; return self.reprocess(); }
            Message::FormantRatioChanged(v)  => { self.formant_ratio   = v; return self.reprocess(); }
            Message::BitDepthChanged(v)      => { self.bit_depth       = v; return self.reprocess(); }
            Message::SampleRateDivChanged(v) => { self.sample_rate_div = v; return self.reprocess(); }

            // DSP result arrives from background thread.
            Message::ProcessingDone(proc_gen, processed, handle) => {
                // Discard stale results from superseded slider positions.
                if proc_gen == self.processing_gen {
                    self.spectrogram_handle = handle;
                    self.processed          = Some(*processed);
                    self.status             = self.file_status.clone();
                }
            }

            // ── Transport ─────────────────────────────────────────────────────
            Message::PlayPause => {
                if self.is_playing {
                    if let Some((start, offset)) = self.play_start {
                        let sr = self.audio.as_ref().map(|a| a.sample_rate as f32).unwrap_or(44_100.0);
                        let total = self.processed.as_ref().map(|p| p.samples.len()).unwrap_or(0);
                        self.play_position =
                            (offset + (start.elapsed().as_secs_f32() * sr) as usize).min(total);
                    }
                    self.stop_playback();
                } else {
                    self.start_playback();
                }
            }
            Message::Stop => {
                self.stop_playback();
                self.play_position = 0;
            }
            Message::Seek(frac) => {
                let total = self.processed.as_ref().map(|p| p.samples.len()).unwrap_or(0);
                self.play_position = (frac * total as f32) as usize;
                if self.is_playing { self.start_playback(); }
            }
            Message::PlayTick => {
                if let Some((start, offset)) = self.play_start {
                    let sr    = self.audio.as_ref().map(|a| a.sample_rate as f32).unwrap_or(44_100.0);
                    let total = self.processed.as_ref().map(|p| p.samples.len()).unwrap_or(0);
                    let new_pos = offset + (start.elapsed().as_secs_f32() * sr) as usize;
                    if new_pos >= total {
                        self.stop_playback();
                        self.play_position = 0;
                    } else {
                        self.play_position = new_pos;
                    }
                }
            }
        }
        Task::none()
    }

    // ── View ──────────────────────────────────────────────────────────────────

    fn view(&self) -> Element<'_, Message> {
        let sr = self.audio.as_ref().map(|a| a.sample_rate as f32).unwrap_or(48_000.0);

        let total_samples = self.processed.as_ref().map(|p| p.samples.len()).unwrap_or(0);
        let play_frac = if total_samples > 0 {
            self.play_position as f32 / total_samples as f32
        } else {
            0.0
        };

        let input_samples = self.audio.as_ref().map(|a| a.samples.clone()).unwrap_or_default();
        let (out_samples, in_spec, out_spec, formant_env, pitch_track) =
            self.processed.as_ref().map(|p| (
                p.samples.clone(),
                p.avg_input_spectrum.clone(),
                p.avg_output_spectrum.clone(),
                p.formant_envelope.clone(),
                p.pitch_track_hz.clone(),
            )).unwrap_or_default();

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
                slider(0.0..=1.0_f32, self.pitch_strength, Message::PitchStrengthChanged)
                    .step(0.01_f32),
                text(format!("{:.2}", self.pitch_strength)).size(12),
                text("Octave Shift").size(13),
                slider(-2.0_f32..=2.0_f32, self.octave, Message::OctaveChanged).step(1.0_f32),
                text(format!("{:+}", self.octave as i8)).size(12),
            ]
            .spacing(6)
            .into(),

            AppMode::Vocode => column![
                text("Carrier Waveform").size(13),
                pick_list(&CARRIER_WAVES[..], Some(self.carrier_wave), Message::CarrierWaveChanged),
                text("Carrier Freq (Hz)").size(13),
                slider(50.0_f32..=2000.0_f32, self.carrier_freq, Message::CarrierFreqChanged)
                    .step(1.0_f32),
                text(format!("{:.0} Hz", self.carrier_freq)).size(12),
            ]
            .spacing(6)
            .into(),

            AppMode::FormantShift => column![
                text("Formant Mode").size(13),
                pick_list(
                    &FORMANT_MODES[..],
                    Some(self.formant_mode),
                    Message::FormantModeChanged,
                ),
                text("Formant Ratio").size(13),
                slider(0.25_f32..=4.0_f32, self.formant_ratio, Message::FormantRatioChanged)
                    .step(0.05_f32),
                text(format!("{:.2}×", self.formant_ratio)).size(12),
            ]
            .spacing(6)
            .into(),

            AppMode::Bitcrusher => column![
                text("Bit Depth").size(13),
                slider(1.0_f32..=16.0_f32, self.bit_depth, Message::BitDepthChanged)
                    .step(1.0_f32),
                text(format!("{} bit", self.bit_depth as i8)).size(12),
                text("Sample Rate ÷").size(13),
                slider(
                    1.0_f32..=32.0_f32,
                    self.sample_rate_div,
                    Message::SampleRateDivChanged,
                )
                .step(1.0_f32),
                text(format!("÷{}", self.sample_rate_div as i8)).size(12),
            ]
            .spacing(6)
            .into(),
        };

        // ── Controls panel ────────────────────────────────────────────────────
        let controls = column![
            button("Open WAV file").on_press(Message::OpenFile),
            mode_row,
            mode_controls,
        ]
        .spacing(12)
        .padding(16)
        .width(230);

        // ── Transport bar ─────────────────────────────────────────────────────
        let has_audio  = total_samples > 0;
        let play_label = if self.is_playing { "Pause" } else { "Play" };

        let play_btn = if has_audio {
            button(play_label).on_press(Message::PlayPause)
        } else {
            button(play_label)
        };
        let stop_btn = if has_audio {
            button("Stop").on_press(Message::Stop)
        } else {
            button("Stop")
        };

        let transport = row![
            play_btn,
            stop_btn,
            slider(0.0_f32..=1.0_f32, play_frac, Message::Seek)
                .step(0.0005_f32)
                .width(Length::Fill),
            text(format!(
                "{} / {}",
                fmt_time(self.play_position, sr as u32),
                fmt_time(total_samples, sr as u32),
            ))
            .size(12),
        ]
        .spacing(8)
        .padding([4, 0]);

        let out_label = format!("Output  ({})", self.app_mode);

        // ── Waveforms + spectra ───────────────────────────────────────────────
        let panels = row![
            column![
                text("Input").size(12),
                Canvas::new(WaveformView { samples: input_samples, color: GREEN, play_frac })
                    .width(Length::Fill).height(90),
                text("Input · Spectrum").size(12),
                Canvas::new(SpectrumView {
                    spectrum: in_spec, formant_env: vec![], color: ORANGE, sample_rate: sr,
                })
                .width(Length::Fill).height(130),
            ]
            .spacing(3)
            .width(Length::FillPortion(1)),

            column![
                text(out_label).size(12),
                Canvas::new(WaveformView { samples: out_samples, color: CYAN, play_frac })
                    .width(Length::Fill).height(90),
                text("Output · Spectrum  + formant envelope").size(12),
                Canvas::new(SpectrumView {
                    spectrum: out_spec, formant_env, color: PURPLE, sample_rate: sr,
                })
                .width(Length::Fill).height(130),
            ]
            .spacing(3)
            .width(Length::FillPortion(1)),
        ]
        .spacing(6);

        // ── Spectrogram ───────────────────────────────────────────────────────
        let spectrogram =
            iced::widget::image::Image::<iced::widget::image::Handle>::new(
                self.spectrogram_handle.clone(),
            )
            .width(Length::Fill)
            .height(160u16)
            .content_fit(ContentFit::Fill);

        // ── Pitch track ───────────────────────────────────────────────────────
        let pitch_panel = Canvas::new(PitchTrackView {
            pitch_track_hz: pitch_track,
            sample_rate:    sr,
        })
        .width(Length::Fill)
        .height(100);

        column![
            row![
                controls,
                column![
                    transport,
                    panels,
                    text("Spectrogram  (log freq, time →)").size(12),
                    spectrogram,
                    text("Pitch track  (YIN, 80–800 Hz)").size(12),
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
    let sr   = sample_rate.max(1) as usize;
    let secs = samples / sr;
    let frac = (samples % sr) * 10 / sr;
    format!("{}:{:02}.{}", secs / 60, secs % 60, frac)
}

fn bin_for_pixel(px: f32, width: f32, n_bins: usize, sample_rate: f32) -> usize {
    let log_min = 20.0_f32.log2();
    let log_max = (sample_rate / 2.0).log2();
    let freq    = 2.0_f32.powf(log_min + (px / width) * (log_max - log_min));
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

fn build_spectrogram_image(
    frames: &[Vec<f32>],
    sample_rate: f32,
) -> iced::widget::image::Handle {
    const IMG_W: u32 = 1024;
    const IMG_H: u32 = 200;

    if frames.is_empty() || frames[0].is_empty() {
        return empty_image();
    }

    let n_frames   = frames.len();
    let n_bins     = frames[0].len();
    let global_max = frames
        .iter()
        .flat_map(|f| f.iter().copied())
        .fold(1e-10_f32, f32::max);

    let mut pixels = vec![0u8; (IMG_W * IMG_H * 4) as usize];

    for py in 0..IMG_H {
        let t       = 1.0 - py as f32 / IMG_H as f32;
        let log_min = 20.0_f32.log2();
        let log_max = (sample_rate / 2.0).log2();
        let freq    = 2.0_f32.powf(log_min + t * (log_max - log_min));
        let bin     = ((freq * 2.0 * n_bins as f32 / sample_rate) as usize).min(n_bins - 1);

        for px in 0..IMG_W {
            let fi   = (px as usize * n_frames / IMG_W as usize).min(n_frames - 1);
            let mag  = frames[fi].get(bin).copied().unwrap_or(0.0);
            let db   = 20.0 * (mag / global_max).max(1e-6).log10();
            let norm = ((db + 70.0) / 70.0).clamp(0.0, 1.0);
            let [r, g, b] = spectrogram_color(norm);
            let idx  = ((py * IMG_W + px) * 4) as usize;
            pixels[idx]     = r;
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
    samples:   Vec<f32>,
    color:     Color,
    play_frac: f32,
}

impl canvas::Program<Message> for WaveformView {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: iced::Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry<Renderer>> {
        let mut frame = Frame::new(renderer, bounds.size());
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), PANEL);

        let cy = bounds.height / 2.0;
        let w  = bounds.width;

        frame.stroke(
            &Path::line(Point::new(0.0, cy), Point::new(w, cy)),
            Stroke::default().with_color(GRID).with_width(1.0),
        );

        if !self.samples.is_empty() {
            let n    = self.samples.len();
            let path = Path::new(|b| {
                for px in 0..(w as usize) {
                    let idx = (px * n / w as usize).min(n - 1);
                    let y   = cy - self.samples[idx].clamp(-1.0, 1.0) * cy * 0.88;
                    if px == 0 { b.move_to(Point::new(0.0, y)); }
                    else       { b.line_to(Point::new(px as f32, y)); }
                }
            });
            frame.stroke(&path, Stroke::default().with_color(self.color).with_width(1.0));
        }

        // Playback cursor
        let cx = self.play_frac * w;
        frame.stroke(
            &Path::line(Point::new(cx, 0.0), Point::new(cx, bounds.height)),
            Stroke::default()
                .with_color(Color { r: 1.0, g: 1.0, b: 1.0, a: 0.7 })
                .with_width(1.5),
        );

        vec![frame.into_geometry()]
    }
}

// ── Spectrum canvas ───────────────────────────────────────────────────────────

struct SpectrumView {
    spectrum:    Vec<f32>,
    formant_env: Vec<f32>,
    color:       Color,
    sample_rate: f32,
}

impl canvas::Program<Message> for SpectrumView {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: iced::Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry<Renderer>> {
        let mut frame = Frame::new(renderer, bounds.size());
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), PANEL);

        let w  = bounds.width;
        let h  = bounds.height;
        let sr = self.sample_rate;
        let n  = self.spectrum.len();

        const FREQ_LABELS: &[(f32, &str)] = &[
            (100.0,    "100"),
            (1_000.0,  "1k"),
            (5_000.0,  "5k"),
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
                    content:              label.to_string(),
                    position:             Point::new(x, h - 14.0),
                    color:                DIM,
                    size:                 Pixels(10.0),
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
                content:  format!("{db}"),
                position: Point::new(2.0, y - 10.0),
                color:    DIM,
                size:     Pixels(10.0),
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
                b.line_to(Point::new(px as f32, h * (1.0 - db_norm(self.spectrum[bin]))));
            }
            b.line_to(Point::new(w, h));
            b.close();
        });
        frame.fill(&fill_path, Color { a: 0.25, ..c });

        let line_path = Path::new(|b| {
            for px in 0..(w as usize) {
                let bin = bin_for_pixel(px as f32, w, n, sr);
                let y   = h * (1.0 - db_norm(self.spectrum[bin]));
                if px == 0 { b.move_to(Point::new(0.0, y)); }
                else       { b.line_to(Point::new(px as f32, y)); }
            }
        });
        frame.stroke(&line_path, Stroke::default().with_color(c).with_width(1.5));

        if !self.formant_env.is_empty() {
            let env_path = Path::new(|b| {
                for px in 0..(w as usize) {
                    let bin = bin_for_pixel(px as f32, w, self.formant_env.len(), sr);
                    let y   = h * (1.0 - db_norm(self.formant_env[bin]));
                    if px == 0 { b.move_to(Point::new(0.0, y)); }
                    else       { b.line_to(Point::new(px as f32, y)); }
                }
            });
            frame.stroke(
                &env_path,
                Stroke::default().with_color(FORMANT_COLOR).with_width(2.0),
            );
        }

        vec![frame.into_geometry()]
    }
}

// ── Pitch track canvas ────────────────────────────────────────────────────────

struct PitchTrackView {
    pitch_track_hz: Vec<f32>,
    sample_rate:    f32,
}

impl canvas::Program<Message> for PitchTrackView {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: iced::Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry<Renderer>> {
        let mut frame = Frame::new(renderer, bounds.size());
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), PANEL);

        let w  = bounds.width;
        let h  = bounds.height;
        let sr = self.sample_rate;

        const NOTE_GRID: &[(f32, &str)] = &[
            (65.4,  "C2"),
            (130.8, "C3"),
            (261.6, "C4"),
            (440.0, "A4"),
            (523.3, "C5"),
            (880.0, "C6"),
        ];
        for &(freq, label) in NOTE_GRID {
            let y = freq_to_y(freq, sr, h);
            if y > 0.0 && y < h {
                frame.stroke(
                    &Path::line(Point::new(0.0, y), Point::new(w, y)),
                    Stroke::default().with_color(GRID).with_width(0.5),
                );
                frame.fill_text(canvas::Text {
                    content:  label.to_string(),
                    position: Point::new(2.0, y - 10.0),
                    color:    DIM,
                    size:     Pixels(10.0),
                    ..canvas::Text::default()
                });
            }
        }

        if self.pitch_track_hz.is_empty() {
            return vec![frame.into_geometry()];
        }

        let n    = self.pitch_track_hz.len();
        let path = Path::new(|b| {
            let mut in_seg = false;
            for (i, &hz) in self.pitch_track_hz.iter().enumerate() {
                if hz < 50.0 { in_seg = false; continue; }
                let x = i as f32 / n as f32 * w;
                let y = freq_to_y(hz, sr, h);
                if !in_seg { b.move_to(Point::new(x, y)); in_seg = true; }
                else       { b.line_to(Point::new(x, y)); }
            }
        });
        frame.stroke(&path, Stroke::default().with_color(GREEN).with_width(2.0));

        vec![frame.into_geometry()]
    }
}
