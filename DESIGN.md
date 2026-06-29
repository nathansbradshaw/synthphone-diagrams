# SynthPhone DSP Viewer — Design Document

## Purpose

An interactive desktop application that visualises the SynthPhone's audio DSP pipeline in real time. The viewer lets you dial in pitch correction, vocoding, formant shifting, and bitcrushing with sliders and hear/see the results instantly. Primary audiences: developers understanding how the hardware synth works, and anyone demoing the SynthPhone's signal chain.

---

## Technology Stack

| Concern | Choice | Reason |
|---|---|---|
| Language | Rust (2024 edition) | Same as the DSP library; no FFI boundary |
| GUI framework | `iced` 0.13 | Native Rust, composable widgets, canvas support for custom drawing |
| Audio output | `rodio` | Plays the DSP thread's processed sample stream (file mode + mic monitor) |
| Audio input | `cpal` | Cross-platform mic capture, feeds the same DSP pipeline as file playback |
| MIDI parsing | `midly` | Parses Standard MIDI Files for Harmony mode's note timeline |
| DSP | `synthphone_e_vocal_dsp` (path dep) | Production-ready pitch correction, vocoder, formant, bitcrusher |
| FFT (visualization) | `microfft` (already in DSP lib) | Reuse what's already there |
| Shared state | `std::sync::{Arc, Mutex}` + `atomic` | Simple, no extra deps |

---

## Cargo.toml Dependencies

```toml
[dependencies]
iced = { version = "0.13", features = ["canvas", "image", "tokio"] }
hound = "3.5"             # WAV file loading
rfd = "0.14"              # native file picker (WAV + MIDI)
rodio = "0.19"            # audio output (file playback + mic monitor)
cpal = "0.15"             # mic capture for live input mode
midly = "0.5"             # Standard MIDI File parsing for Harmony mode
tokio = { version = "1", features = ["rt-multi-thread"] }
synthphone_e_vocal_dsp = { path = "../synthphone_e_vocal_dsp", package = "synthphone-e-vocal-dsp" }
```

---

## Architecture

### Thread Model

```
┌─────────────────────────────────────────────────────────────┐
│  GUI Thread (iced main loop, ~60 fps)                       │
│   └── Subscription polls VisualizationSnapshot              │
│       └── on tick: snapshot → App state → re-render        │
└───────────────────────────┬─────────────────────────────────┘
                            │  Arc<Mutex<VisualizationSnapshot>>
                            │  Arc<Mutex<DspParams>>            (GUI → DSP)
                            │  Arc<AtomicBool> stop / bypass_until: Instant
┌───────────────────────────▼─────────────────────────────────┐
│  DSP Thread (dsp_loop, one STFT hop at a time)               │
│   1. Pull next 1024-sample block from the active source:     │
│      • File mode      — pre-loaded Vec<f32>, advances by hop │
│      • Microphone mode — cpal input ring buffer, blocks until│
│        a full hop has arrived (paces the loop to real time)  │
│   2. If `now < bypass_until`: skip DSP, pass block through    │
│      unprocessed (see "Temporary Effects Bypass")             │
│   3. Else run bitcrush/sample-rate-reduce → process_vocal_    │
│      effects_1024() → OLA-add into output accumulator        │
│      (Harmony mode additionally pulls the current target      │
│      pitch from the MIDI timeline — see "Harmony Mode")       │
│   4. find_pitch_yin() on both raw input and processed output  │
│   5. Write input/output windows, FFTs, both spectrogram cols, │
│      both pitch values → VisualizationSnapshot                │
│   6. Push processed samples to the audio_tx channel → rodio    │
└─────────────────────────────────────────────────────────────┘
```

In **File mode** the DSP thread paces itself against a bounded `mpsc::sync_channel` that rodio drains, so it naturally tracks real-time playback speed without an explicit clock. In **Microphone mode** there is no fixed-length source to loop over: the cpal input callback pushes captured samples into a ring buffer and the DSP thread blocks until a full hop is available, so the same `dsp_loop` body runs unmodified — only where it *reads its input block* changes.

### Shared Data Structure

```rust
/// Written by the DSP thread each hop; read by the GUI thread each tick.
/// Stored as Arc<Mutex<VisualizationSnapshot>> — the GUI takes the lock
/// briefly to copy the fields it needs into its RenderSnapshot.
pub struct VisualizationSnapshot {
    // Time domain (current 1024-sample STFT window)
    pub input_window:       Vec<f32>,
    pub output_window:      Vec<f32>,

    // Frequency domain (latest FFT frame, 512 bins)
    pub input_fft:          Vec<f32>,          // magnitude spectrum
    pub output_fft:         Vec<f32>,
    pub formant_envelope:   Vec<f32>,          // cepstral envelope overlay

    // Spectrogram (one column per STFT frame — kept separate for in/out)
    pub input_spec_col:     Vec<f32>,          // latest input  FFT magnitudes
    pub output_spec_col:    Vec<f32>,          // latest output FFT magnitudes
    pub col_serial:         u64,               // increments every frame

    // Pitch tracking (Hz per STFT frame, GUI accumulates into rolling history)
    pub input_pitch_hz:     f32,
    pub output_pitch_hz:    f32,

    // Diagnostics / playback
    pub play_position:      usize,
    pub total_samples:      usize,
}

/// GUI-side rolling buffers (App owns these; DSP thread does not touch them)
struct RenderSnapshot {
    input_waveform:          VecDeque<f32>,
    output_waveform:         VecDeque<f32>,
    input_pitch_history:     VecDeque<f32>,   // Hz, last ~3 s of input pitch
    output_pitch_history:    VecDeque<f32>,   // Hz, last ~3 s of output pitch
    input_spectrogram_cols:  VecDeque<Vec<f32>>,
    output_spectrogram_cols: VecDeque<Vec<f32>>,
    // ... plus copies of fft/formant for immediate draw
}
```

---

## Source Layout

```
src/
  main.rs               — iced app entry point, App struct, Message enum,
                          update(), view(), RenderSnapshot accumulation
  audio.rs              — WAV file loading (hound), DspParams struct
  realtime.rs           — dsp_loop: STFT pipeline, VisualizationSnapshot,
                          ChannelSource (rodio source backed by sync_channel)
  mic.rs                — cpal mic capture: opens the default input device,
                          pushes samples into a ringbuf SPSC ring that the
                          DSP thread drains instead of a pre-loaded Vec<f32>
  midi.rs               — MIDI file loading (midly), MidiTimeline struct:
                          maps playback sample-position → current target note
                          for Harmony mode
  ui/
    theme.rs            — Color palette constants
    graphs/
      waveform.rs       — Canvas: input waveform (green)
      waveform_out.rs   — Canvas: output waveform (cyan)
      spectrum.rs       — Canvas: FFT magnitude + formant envelope overlay
      spectrogram.rs    — Canvas: rolling input spectrogram (hot colormap)
      spectrogram_out.rs— Canvas: rolling output spectrogram
      pitch_track.rs    — Canvas: input pitch (YIN) over time (green)
      pitch_track_out.rs— Canvas: output pitch (YIN) over time (cyan)
      cepstrum.rs       — Canvas: cepstrum + formant lifter markers
```

---

## Layout

### Wireframe

```
┌────────────────────────────────────────────────────────────────────────┐
│  SynthPhone DSP Viewer                                                 │
│  [Pitch Correction ▼]  Key:[C ▼]  Scale:[Major ▼]  Source:[File ▼]   │  ← Header bar
│  [Open WAV…]  [Open MIDI…]  ◀ Seek bar ▶  [Play/Pause]  [Bypass Now]  │  ← Transport
├──────────────────┬─────────────────────────────────────────────────────┤
│                  │  ┌─────────────────────┬─────────────────────────┐ │
│  CONTROLS        │  │  Input Waveform     │  Output Waveform        │ │
│                  │  │  (green, raw)       │  (cyan, processed)      │ │
│  Pitch Strength  │  ├─────────────────────┼─────────────────────────┤ │
│  ░░░░░░●░░░ 0.8  │  │  Input FFT          │  Output FFT             │ │
│                  │  │  (orange bars)      │  (orange + purple env)  │ │
│  Octave          │  ├─────────────────────┼─────────────────────────┤ │
│  ░░●░░░░░░ +0    │  │  Input Spectrogram  │  Output Spectrogram     │ │
│                  │  │  (hot colormap)     │  (hot colormap)         │ │
│  Formant         │  ├─────────────────────┼─────────────────────────┤ │
│  ●░░░░░░░░ 0.5×  │  │  Input Pitch (YIN)  │  Output Pitch (YIN)     │ │
│  (Male)          │  │  (green, Hz/time)   │  (cyan, Hz/time)        │ │
│                  │  │  note grid + labels │  note grid + labels     │ │
│  Bit Depth       │  ├─────────────────────┴─────────────────────────┤ │
│  ░░░░░░░░●  16   │  │  Cepstrum (quefrency, formant peaks, yellow)  │ │
│                  │  └───────────────────────────────────────────────┘ │
│  Sample Rate ÷   │                                                     │
│  ●░░░░░░░░░  1×  │                                                     │
│                  │                                                     │
│  FFT Size        │                                                     │
│  [ 1024 ▼ ]      │                                                     │
│                  │                                                     │
│  [Mute Out ☐]    │                                                     │
├──────────────────┴─────────────────────────────────────────────────────┤
│  ● 48 kHz · FFT 1024 · Hop 256 · ~21ms latency · CPU 3% · Note: A4   │  ← Status bar
└────────────────────────────────────────────────────────────────────────┘
```

### Panel Breakdown

| Panel | Size | Content |
|---|---|---|
| Header | full width × 48px | Mode picker, key/scale pickers, source toggle (File/Mic) |
| Transport | full width × 36px | Open WAV, Open MIDI, seek bar, Play/Pause, **Bypass Now** button |
| Controls (left) | 240px × fill | Mode-specific sliders, global settings, Mute Out |
| Input Waveform | 50% × 120px | Raw input (file or mic), last 1024 samples, green |
| Output Waveform | 50% × 120px | Processed audio, last 1024 samples, cyan |
| Input FFT | 50% × 160px | Magnitude spectrum 0–24 kHz, orange bars, log-x axis |
| Output FFT | 50% × 160px | Same + purple formant envelope overlay, detected pitch marker |
| Input Spectrogram | 50% × 180px | Rolling input FFT heatmap, hot colormap, time → |
| Output Spectrogram | 50% × 180px | Rolling output FFT heatmap, hot colormap, time → |
| Input Pitch Track | 50% × 120px | YIN-detected input F0 (Hz) over last ~3 s, green, note grid |
| Output Pitch Track | 50% × 120px | YIN-detected output F0 (Hz) over last ~3 s, cyan, note grid |
| Cepstrum | full width × 100px | Quefrency domain, shows formant structure, yellow |
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

**Harmony mode:**
```
  MIDI file             [ no file loaded ]  [Open MIDI…]
  MIDI Track            [ Track 1 ▼ ]
  Pitch Correction      ░░░░░░●░░ 0.80     (how tightly voice follows MIDI note)
  Formant Lock          [●] On  [ ] Off     (keep vocal character while shifting)
  Octave Offset         [ 0 ▼ ]             (-2 … +2 relative to MIDI note)
  ── MIDI Timeline ──   [●●●●●    ░░░░░░░░]  (read-only playhead visualization)
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

### Spectrogram Canvas (Input)
- Draws `input_spectrogram_cols` from `RenderSnapshot`
- Each STFT frame = one pixel column (or 2px for visibility), newest on the right
- Frequency bins mapped to Y (low=bottom, high=top), log scale
- Colormap: `black → #000044 → #0044ff → #00ffff → #ffff00 → #ff0000` (hot)
- Magnitude normalized to per-frame peak so quiet signals stay visible

### Spectrogram Canvas (Output)
- Identical layout to the input spectrogram but draws `output_spectrogram_cols`
- Uses the same hot colormap; normalized independently so quiet processed frames stay visible
- Lets the user see how the DSP (e.g. vocoder) reshapes the spectrum frame-by-frame

### Cepstrum Canvas
- X axis: quefrency (ms), 0 → ~20ms (fundamental period range 50 Hz–∞)
- Y axis: cepstral amplitude, arbitrary units
- Line plot in `#ffdd44` (yellow)
- Vertical markers at detected formant lifter boundaries (dashed gray)
- Detected F0 quefrency: vertical red marker with frequency label

### Pitch Track Canvas (Input — YIN)
- Draws `input_pitch_history` from `RenderSnapshot`
- X axis: time (last ~3 seconds), scrolling right
- Y axis: frequency in Hz (log scale) with MIDI note grid lines + labels (C3, A4, etc.)
- Line: `#44ff88` (green, matches input waveform color) with circular dot at latest value
- Drop to 0 Hz = unvoiced (line hidden)
- Shows the raw pre-DSP fundamental as detected by `find_pitch_yin`

### Pitch Track Canvas (Output — YIN)
- Draws `output_pitch_history` from `RenderSnapshot`
- Same axes and note grid as input canvas (same Y scale for direct comparison)
- Line: `#44ddff` (cyan, matches output waveform color) with circular dot at latest value
- Corrected pitch target: `#44ff88` dim horizontal band showing pitch-correction target note (Harmony mode: band follows the MIDI note at current playback position)
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

## Temporary Effects Bypass

A **"Bypass Now"** button in the transport bar temporarily disables all DSP effects for a fixed window (default: 1 second) so you can hear the unprocessed signal and compare it with the processed output.

### Behaviour
- Press the button → sets `bypass_until: Instant::now() + Duration::from_secs(1)` on the `DspParams` struct (shared with the DSP thread).
- The DSP thread checks `now < bypass_until` at the top of each hop. When true it skips `process_vocal_effects_1024` and passes the raw input block directly to the output OLA accumulator. Pitch detection still runs so the visualizations keep updating.
- The transport bar shows a countdown (e.g. "Bypassing… 0.6s") that ticks down to zero.
- Re-pressing the button resets the timer to 1 second from now (so repeated presses extend the bypass window).
- After the timer expires the DSP thread returns to normal processing transparently — no click/pop because the OLA overlap handles continuity.

### App state additions
```rust
// DspParams (on the DSP thread side)
pub bypass_until: Option<std::time::Instant>,   // None = not bypassing

// Message (GUI side)
BypassNowPressed,       // user pressed the button
BypassTimerTick,        // subscription fires ~10× per second to update countdown
```

---

## Harmony Mode (MIDI-driven Pitch Targeting)

Harmony mode replicates the SynthPhone hardware's MIDI-driven voice correction: as a MIDI file plays alongside the audio, each MIDI note event sets the pitch-correction target so the vocalist's voice is pulled toward the melody/harmony line note-by-note.

### MIDI Timeline

`midi.rs` loads a Standard MIDI File with `midly` and builds a `MidiTimeline`:

```rust
pub struct MidiTimeline {
    /// Sorted list of (sample_offset, target_midi_note) events.
    /// sample_offset is computed from MIDI ticks using the file's tempo map.
    pub events: Vec<(usize, u8)>,  // (sample offset, MIDI note 0-127)
}

impl MidiTimeline {
    /// Returns the active MIDI note at a given playback sample position.
    pub fn note_at(&self, sample_pos: usize) -> Option<u8> { ... }
}
```

The MIDI file's tempo/tick resolution and the audio sample rate both need to be known to convert MIDI ticks to sample offsets. The timeline is pre-computed on load so the DSP thread never touches the MIDI file directly — it only calls `timeline.note_at(play_position)` on each hop.

### DSP Integration

In Harmony mode the DSP thread reads the current target note from the timeline before calling `process_vocal_effects_1024`:

```
  let target_note = midi_timeline.note_at(play_position);
  cur.settings.target_note = target_note;   // MusicalSettings field
  // then run process_vocal_effects_1024 as normal
```

Between MIDI note events the target stays at the last note (gate-hold behaviour, same as the phone). When no MIDI note is active (e.g. rests) the pitch correction can be disabled or held at the last note — user-selectable.

### Visualizer additions

- The Output Pitch Track canvas draws a step-function overlay in a dim accent color showing the active MIDI target note over time, so users can see how closely the voice tracked the MIDI line.
- The MIDI Timeline readout in the Harmony controls panel shows a horizontal scrolling bar of upcoming note events.
- In status bar: "MIDI: C4" shows the current MIDI target note.

### Synchronisation with Microphone mode

In Mic mode there is no fixed `play_position` from a file. Instead a real-time clock drives the MIDI timeline position starting from when playback begins:

```rust
let elapsed_samples = start_time.elapsed().as_secs_f32() * sample_rate;
let target_note = midi_timeline.note_at(elapsed_samples as usize);
```

This means the MIDI file and mic are both "live" — useful for live performance with a backing MIDI track.

---

## Microphone Input Mode

A **Source** dropdown in the header bar toggles between `File` and `Microphone`. Both sources feed the same `dsp_loop` body; only the input provider changes.

### Architecture

`mic.rs` owns a `cpal` input stream. When Microphone mode is selected:

1. `cpal::host().default_input_device()` opens the mic at the closest supported rate to 48 kHz.
2. The `cpal` callback writes captured samples into a `ringbuf` SPSC ring buffer.
3. `dsp_loop` is changed to read its input block from the ring buffer instead of the pre-loaded `Vec<f32>`:

```rust
// File mode (current)
block.copy_from_slice(&padded[pos..pos + N]);

// Mic mode (new)
// Blocks until N samples are available in the ring (paces the loop to real-time)
while ring_consumer.len() < N { std::hint::spin_loop(); }
ring_consumer.pop_slice(&mut block);
```

4. `play_position` increments by `hop` each frame (monotonic counter) rather than tracking a file offset.
5. End-of-file / looping logic is skipped; the loop runs until the stop flag is set.

### Seamless mode switching

- Switching to Mic stops the current DSP thread (`stop.store(true)`), drops the file's `ChannelSource`, opens the cpal stream, and starts a new DSP thread passing a `MicRingConsumer` instead of a `Vec<f32>`.
- The visualization buffers (spectrogram history, pitch history) are cleared on the switch to avoid mixing file and mic data in the rolling displays.
- rodio is still used for monitor output (processed mic signal → speakers). A "Monitor" toggle controls whether the processed signal is sent to the output device (avoids feedback).

### New Message variants

```rust
SourceChanged(AudioSource),   // File / Microphone
MidiFileOpened(Option<PathBuf>),
HarmonyStrengthChanged(f32),
HarmonyFormantLockToggled(bool),
HarmonyOctaveOffsetChanged(i8),
BypassNowPressed,
```

---

## iced App Skeleton

```rust
// main.rs

struct App {
    mode: ProcessingMode,
    source: AudioSource,           // File | Microphone
    config: VocalEffectsConfig,
    musical: MusicalSettings,
    midi_timeline: Option<Arc<MidiTimeline>>,
    viz: Arc<Mutex<VisualizationSnapshot>>,
    render_snapshot: RenderSnapshot,   // no lock held during draw
    bypass_until: Option<std::time::Instant>,
    dsp_params: Arc<Mutex<DspParams>>,
    dsp_stop: Arc<AtomicBool>,
    status: String,
}

#[derive(Debug, Clone)]
enum AudioSource { File, Microphone }

#[derive(Debug, Clone)]
enum Message {
    AudioTick,                         // 60 fps subscription
    VisualizationUpdated(RenderSnapshot),
    // Source & file management
    SourceChanged(AudioSource),
    OpenFile,
    FileOpened(Option<PathBuf>),
    OpenMidi,
    MidiFileOpened(Option<PathBuf>),
    // Mode & musical parameters
    ModeChanged(ProcessingMode),
    PitchStrengthChanged(f32),
    FormantRatioChanged(f32),
    BitDepthChanged(u8),
    SampleRateDivChanged(u32),
    KeyChanged(MusicalKey),
    ScaleChanged(MusicalScale),
    OctaveChanged(i8),
    FftSizeChanged(usize),
    // Transport
    PlayPausePressed,
    BypassNowPressed,
    BypassTimerTick,
    // Harmony mode
    HarmonyStrengthChanged(f32),
    HarmonyFormantLockToggled(bool),
    HarmonyOctaveOffsetChanged(i8),
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
| MIDI target note | `#58a6ff` dim | Harmony target band on output pitch canvas |
| Bypass active | `#ffaa00` | Countdown overlay on transport bar when bypassing |
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

### Phase 6 — Split visualizations & Bypass
- Split `PitchTrackView` into two independent canvases: `pitch_track.rs` (input, green) and `pitch_track_out.rs` (output, cyan), each with their own scrolling history
- Add output spectrogram canvas (`spectrogram_out.rs`) fed from `output_spec_col` in `VisualizationSnapshot`; move existing spectrogram to input-only
- Add `[Bypass Now]` button to transport bar; wire `bypass_until` through `DspParams`; add countdown label in transport bar

### Phase 7 — Microphone input mode
- Add `mic.rs`: open `cpal` default input device, push samples into a `ringbuf` SPSC ring
- Parameterise `dsp_loop` so it accepts an `InputSource` enum (`File(Vec<f32>)` or `Mic(RingConsumer)`) — mic branch blocks on ring until a full hop is available
- Add `Source` dropdown to header bar; wire `SourceChanged` message to stop/restart DSP thread with the correct input
- Add Monitor toggle (bypass output to speakers only when enabled, to prevent feedback on headphones vs. speakers)

### Phase 8 — Harmony mode (MIDI-driven pitch targeting)
- Add `midi.rs`: load `.mid` file with `midly`, build `MidiTimeline` (tick-to-sample conversion with tempo map)
- Add `Harmony` variant to `ProcessingMode` in the DSP lib (or overlay on Pitch Correction mode with MIDI-supplied target note)
- Wire MIDI timeline: DSP thread calls `timeline.note_at(play_position)` each hop and sets `cur.settings.target_note` before `process_vocal_effects_1024`
- In Mic mode: advance MIDI timeline by elapsed real-time samples instead of file `play_position`
- Add Harmony control panel to UI; add MIDI target step-function overlay on Output Pitch Track canvas; add "MIDI: C4" to status bar
