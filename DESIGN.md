# SynthPhone DSP Viewer — Design Document

## Purpose

An interactive desktop application that visualises the SynthPhone's audio DSP pipeline in real time. The viewer lets you dial in pitch correction, vocoding, formant shifting, and bitcrushing with sliders and hear/see the results instantly. Primary audiences: developers understanding how the hardware synth works, and anyone demoing the SynthPhone's signal chain.

---

## Technology Stack

| Concern | Choice | Reason |
|---|---|---|
| Language | Rust (2024 edition) | Same as the DSP library; no FFI boundary |
| GUI framework | `iced` 0.13 | Native Rust, composable widgets, canvas support for custom drawing |
| Audio I/O | `cpal` | Cross-platform mic input and output |
| DSP | `synthphone_e_vocal_dsp` (path dep) | Production-ready pitch correction, vocoder, formant, bitcrusher |
| FFT (visualization) | `microfft` (already in DSP lib) | Reuse what's already there |
| Shared state | `std::sync::{Arc, Mutex}` + `atomic` | Simple, no extra deps |

---

## Cargo.toml Dependencies

```toml
[dependencies]
iced = { version = "0.13", features = ["canvas", "tokio"] }
cpal = "0.15"
synthphone_e_vocal_dsp = { path = "../synthphone_e_vocal_dsp" }
ringbuf = "0.4"          # SPSC ring buffer for audio → GUI data
```

---

## Architecture

### Thread Model

```
┌─────────────────────────────────────────────────────────────┐
│  GUI Thread (iced main loop, ~60 fps)                       │
│   └── Subscription polls SharedVisualizationData           │
│       └── on tick: snapshot → App state → re-render        │
└───────────────────────────┬─────────────────────────────────┘
                            │  Arc<Mutex<SharedVisualizationData>>
                            │  Arc<Mutex<VocalEffectsConfig>>   (GUI → audio)
┌───────────────────────────▼─────────────────────────────────┐
│  Audio Thread (cpal callback, ~48 kHz)                      │
│   1. Read mic block (128 samples)                           │
│   2. Accumulate into 1024-sample STFT input ring            │
│   3. process_vocal_effects_1024() → processed block         │
│   4. Write raw input + processed output → visualization bufs│
│   5. Copy latest FFT frame → SharedVisualizationData        │
│   6. Output processed samples to speakers                   │
└─────────────────────────────────────────────────────────────┘
```

### Shared Data Structure

```rust
pub struct SharedVisualizationData {
    // Time domain (ring buffers holding ~2048 recent samples)
    pub input_waveform:   VecDeque<f32>,
    pub output_waveform:  VecDeque<f32>,

    // Frequency domain (latest FFT frame, 512 bins)
    pub input_fft:        Vec<f32>,          // magnitude spectrum
    pub output_fft:       Vec<f32>,
    pub formant_envelope: Vec<f32>,          // cepstral envelope overlay

    // Cepstrum (quefrency domain)
    pub cepstrum:         Vec<f32>,

    // Pitch tracking (ring buffer of fundamentals over time)
    pub pitch_track:      VecDeque<f32>,     // Hz per STFT frame

    // Spectrogram (rolling: newest column appended, oldest dropped)
    pub spectrogram_cols: VecDeque<Vec<f32>>, // each column = 1 FFT frame

    // Diagnostics
    pub cpu_load_pct:     f32,
    pub detected_note:    Option<String>,    // e.g. "A4"
    pub latency_ms:       f32,
}
```

---

## Source Layout

```
src/
  main.rs               — iced app entry point
  audio/
    mod.rs              — AudioEngine: owns cpal stream + shared state
    processor.rs        — DSP pipeline: calls synthphone_e_vocal_dsp
    shared.rs           — SharedVisualizationData definition
  ui/
    mod.rs              — App struct, Message enum, update(), view()
    theme.rs            — Color palette, custom iced Theme
    controls/
      mod.rs            — mode_controls() dispatcher
      pitch.rs          — Pitch correction sliders
      vocoder.rs        — Vocoder controls
      formant.rs        — Formant shift controls
      bitcrusher.rs     — Bit depth + sample rate reduction sliders
      global.rs         — FFT size, bypass, input gain
    graphs/
      mod.rs
      waveform.rs       — Canvas: time-domain waveform (input & output)
      spectrum.rs       — Canvas: FFT magnitude + formant envelope overlay
      spectrogram.rs    — Canvas: rolling time-frequency heatmap
      cepstrum.rs       — Canvas: cepstrum + pitch track overlay
```

---

## Layout

### Wireframe

```
┌────────────────────────────────────────────────────────────────────────┐
│  SynthPhone DSP Viewer                                                 │
│  [Pitch Correction ▼]  Key: [C ▼]  Scale: [Major ▼]   [● Live]       │  ← Header bar
├──────────────────┬─────────────────────────────────────────────────────┤
│                  │  ┌─────────────────────┬─────────────────────────┐ │
│  CONTROLS        │  │  Input Waveform     │  Output Waveform        │ │
│                  │  │  (green, raw mic)   │  (cyan, processed)      │ │
│  Pitch Strength  │  │                     │                         │ │
│  ░░░░░░●░░░ 0.8  │  ├─────────────────────┼─────────────────────────┤ │
│                  │  │  Input FFT          │  Output FFT             │ │
│  Octave          │  │  (orange bars)      │  (orange + purple       │ │
│  ░░●░░░░░░ +0    │  │                     │   formant envelope)     │ │
│                  │  ├─────────────────────┴─────────────────────────┤ │
│  Formant         │  │  Spectrogram  (time →, freq ↑, hot colormap)  │ │
│  ●░░░░░░░░ 0.5×  │  │                                               │ │
│  (Male)          │  ├─────────────────────┬─────────────────────────┤ │
│                  │  │  Cepstrum           │  Pitch Track            │ │
│  Bit Depth       │  │  (shows formant     │  (detected Hz over      │ │
│  ░░░░░░░░●  16   │  │   peaks as queues)  │   time, note labels)   │ │
│                  │  └─────────────────────┴─────────────────────────┘ │
│  Sample Rate ÷   │                                                     │
│  ●░░░░░░░░░  1×  │                                                     │
│                  │                                                     │
│  FFT Size        │                                                     │
│  [ 1024 ▼ ]      │                                                     │
│                  │                                                     │
│  [Bypass ☐]      │                                                     │
│  [Mute Out ☐]    │                                                     │
├──────────────────┴─────────────────────────────────────────────────────┤
│  ● 48 kHz · FFT 1024 · Hop 256 · ~21ms latency · CPU 3% · Note: A4   │  ← Status bar
└────────────────────────────────────────────────────────────────────────┘
```

### Panel Breakdown

| Panel | Size | Content |
|---|---|---|
| Header | full width × 48px | Mode picker, key/scale pickers, live toggle |
| Controls (left) | 240px × fill | Mode-specific sliders, global settings |
| Input Waveform | 50% × 120px | Raw mic, last 2048 samples, green |
| Output Waveform | 50% × 120px | Processed audio, last 2048 samples, cyan |
| Input FFT | 50% × 160px | Magnitude spectrum 0–24 kHz, orange bars, log-x axis |
| Output FFT | 50% × 160px | Same + purple formant envelope overlay, detected pitch marker |
| Spectrogram | full width × 200px | Rolling heatmap, ~60 columns, hot colormap |
| Cepstrum | 50% × 120px | Quefrency domain, shows formant structure |
| Pitch Track | 50% × 120px | Detected F0 (Hz) over last ~3 seconds, note grid |
| Status bar | full width × 28px | Sample rate, FFT config, latency, CPU, detected note |

### Mode-Specific Control Panels

**Pitch Correction mode:**
```
  Correction Strength   ░░░░░░●░░ 0.80
  Target Key            [ C ▼ ]
  Scale                 [ Major ▼ ]
  Octave Shift          [ 0 ▼ ]   (-2 … +2)
  Speed (smoothing)     ░●░░░░░░░ 0.15
```

**Vocoder mode:**
```
  Carrier Waveform      ( ) Sine  (●) Saw  ( ) Square
  Carrier Note          [ A3 ▼ ]
  Carrier Gain          ░░░░●░░░░ 0.5
  Formant Ratio         ░░●░░░░░░ 0.5×  (Male 0.5 … Female 2.0)
```

**Formant Shift mode:**
```
  Formant Ratio         ░░░●░░░░░ 1.0×
  Preset                [ Neutral ▼ ]   (Male / Female / Neutral / Custom)
  Pitch Shift           ░░░░●░░░░ 0 st  (-12 … +12 semitones)
```

**Bitcrusher mode:**
```
  Bit Depth             ░░░░░░░░● 16 bit   (1 … 16)
  Sample Rate ÷         ●░░░░░░░░ 1×       (1 … 32)
  Pre-gain              ░░░░●░░░░ 0 dB
  Post-gain             ░░░░●░░░░ 0 dB
```

---

## Graph Design Details

### Waveform Canvas
- X axis: sample index (0 → N), no label needed
- Y axis: -1.0 → +1.0, center line drawn in dim gray
- Waveform: single `Path` polyline, anti-aliased
- Colors: input=`#44ff88` (green), output=`#44ddff` (cyan)
- Clipping indicator: axis border flashes red if any sample hits ±1.0

### FFT Spectrum Canvas
- X axis: log-frequency scale, 20 Hz → 24 kHz, labeled at octave intervals
- Y axis: -80 dB → 0 dB (magnitudes in dB, floor at -80)
- Input bars: `#ff8844` (orange), filled rectangles
- Formant envelope (output panel only): `#cc44ff` (purple) smooth curve drawn over bars
- Detected pitch: vertical dashed line at F0 frequency, labeled with note name
- Harmonics overlay: dim markers at 2×, 3×, 4× F0 to show harmonic series

### Spectrogram Canvas
- Each STFT frame = one pixel column (or 2px for visibility), prepended left
- Frequency bins mapped to Y (low=bottom, high=top), log scale
- Colormap: `black → #000044 → #0044ff → #00ffff → #ffff00 → #ff0000` (hot)
- Magnitude normalized to per-frame peak so quiet signals stay visible
- Pitch track line overlaid in white

### Cepstrum Canvas
- X axis: quefrency (ms), 0 → ~20ms (fundamental period range 50 Hz–∞)
- Y axis: cepstral amplitude, arbitrary units
- Line plot in `#ffdd44` (yellow)
- Vertical markers at detected formant lifter boundaries (dashed gray)
- Detected F0 quefrency: vertical red marker with frequency label

### Pitch Track Canvas
- X axis: time (last ~3 seconds), scrolling right
- Y axis: frequency in Hz (log scale) with MIDI note grid lines + labels (C3, A4, etc.)
- Line: `#ffffff` with circular dot at latest value
- Corrected pitch target: `#44ff88` horizontal band (pitch-correction target note)
- Drop to 0 Hz = unvoiced (line hidden)

---

## DSP Pipeline (per STFT block)

```
Mic input (cpal callback, 128 samples)
  │
  ▼
Input ring accumulator (fills to block_size)
  │
  ├──► snapshot raw samples → SharedVisualizationData.input_waveform
  │
  ▼
Apply pre-effects:
  ├── bitcrush(sample, bit_depth)          if mode == Bitcrusher
  └── sample_rate_reduce(sample, factor)   if mode == Bitcrusher
  │
  ▼
process_vocal_effects_1024(
    &input_block,
    &mut output_block,
    &musical_settings,
    &config,
)
  │
  ├──► snapshot output + FFT frame → SharedVisualizationData
  │
  ▼
Output ring → cpal output callback → speakers
```

The `VocalEffectsConfig` and `MusicalSettings` structs come from the DSP lib and are rebuilt from UI slider values on every change via `Arc<Mutex<>>`.

---

## iced App Skeleton

```rust
// ui/mod.rs

struct App {
    mode: ProcessingMode,
    config: VocalEffectsConfig,
    musical: MusicalSettings,
    viz: Arc<Mutex<SharedVisualizationData>>,
    // snapshot for render (no lock held during draw)
    render_snapshot: RenderSnapshot,
}

#[derive(Debug, Clone)]
enum Message {
    AudioTick(RenderSnapshot),   // from Subscription
    ModeChanged(ProcessingMode),
    PitchStrengthChanged(f32),
    FormantRatioChanged(f32),
    BitDepthChanged(u8),
    SampleRateDivChanged(u32),
    KeyChanged(MusicalKey),
    ScaleChanged(MusicalScale),
    OctaveChanged(i8),
    BypassToggled(bool),
    FftSizeChanged(usize),
}

impl iced::Application for App {
    fn subscription(&self) -> Subscription<Message> {
        // 60fps poll of shared viz data
        iced::time::every(Duration::from_millis(16))
            .map(|_| snapshot_viz(&self.viz))
            .map(Message::AudioTick)
    }

    fn update(&mut self, msg: Message) -> Command<Message> {
        match msg {
            Message::PitchStrengthChanged(v) => {
                self.config.pitch_correction_strength = v;
                // push to audio thread via Arc<Mutex<VocalEffectsConfig>>
            }
            // … etc
        }
    }

    fn view(&self) -> Element<Message> {
        column![
            header_bar(self),
            row![
                controls_panel(self),
                graphs_panel(&self.render_snapshot),
            ],
            status_bar(&self.render_snapshot),
        ].into()
    }
}
```

---

## Color Palette

| Name | Hex | Used for |
|---|---|---|
| Background | `#0d1117` | Window background |
| Surface | `#161b22` | Panel backgrounds |
| Border | `#30363d` | Panel borders, axes |
| Input waveform | `#44ff88` | Raw mic signal |
| Output waveform | `#44ddff` | Processed signal |
| FFT bars | `#ff8844` | Magnitude spectrum |
| Formant envelope | `#cc44ff` | Formant curve overlay |
| Pitch track | `#ffffff` | Detected F0 line |
| Pitch target | `#44ff88` | Correction target note |
| Cepstrum | `#ffdd44` | Quefrency domain |
| Text primary | `#e6edf3` | Labels, values |
| Text secondary | `#8b949e` | Axis labels, hints |
| Accent | `#58a6ff` | Slider thumbs, focus |
| Warning | `#ff6b6b` | Clip indicator |

---

## Implementation Phases

### Phase 1 — Audio plumbing
- Set up `cpal` mic input + output stream
- Wire `synthphone_e_vocal_dsp` processing in the audio callback
- Populate `SharedVisualizationData` (waveforms + FFT frame)
- Verify audio round-trip works (bypass mode)

### Phase 2 — Basic UI skeleton
- iced app with header, left panel, right panel layout
- Mode picker + key/scale pickers
- Pitch correction sliders wired to `VocalEffectsConfig`
- Waveform canvas (input + output) rendering live

### Phase 3 — FFT & spectral graphs
- FFT magnitude spectrum canvas (input + output panels)
- Formant envelope overlay (cepstral lifter output)
- Cepstrum canvas

### Phase 4 — Spectrogram + pitch track
- Rolling spectrogram with hot colormap
- Pitch track canvas with note grid
- Detected note display in status bar

### Phase 5 — Remaining modes & polish
- Vocoder controls + carrier oscillator wiring
- Formant shift controls
- Bitcrusher controls
- Theme polish, axis labels, tooltips on sliders
- CPU load measurement
