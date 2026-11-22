# DSP Viewer · Web instrument

**[Open DSP Viewer](https://synthphone-dsp-viewer.pinteru.chatgpt.site)** · owner-private hosting

1. **Power on** to start the harmonic test oscillator.
2. Choose an effect. Green shows input; amber shows actual processed output.
3. Switch the screen between waveform, spectrum, spectrogram, pitch, and measured spectral envelope.
4. Enable **Listen** to hear output; use headphones with a microphone. Microphone monitoring always starts muted.

Audio and MIDI files stay in the browser. No account, service, or upload participates in audio processing. The hosted page itself uses the hosting provider's private access controls.

## Build / preview

Install Rust's `wasm32-unknown-unknown` target and `wasm-bindgen-cli` **0.2.127**. No Node, npm, Python, or frontend framework is required to build or serve the app.

From `web/`:

```sh
cargo run --manifest-path xtask/Cargo.toml -- build
cargo run --manifest-path xtask/Cargo.toml -- serve 3040
```

Open `http://127.0.0.1:3040`. The server binds only to loopback. Ctrl-C stops it. `dist/` is disposable and ignored. To reuse an existing Cargo cache for both build tools and WASM, set `CARGO_TARGET_DIR` to its absolute path before these commands.

```sh
cargo test --locked
cargo fmt -- --check
cargo fmt --manifest-path xtask/Cargo.toml -- --check
```

The DSP dependency is pinned to a published Git revision; no sibling library checkout is needed for the web build.

## Efficiency

- Rust owns DSP, MIDI parsing, controls, plotting, and the build/server tool.
- A minimal JavaScript `AudioWorkletProcessor` bridges browser PCM to a separate Rust WASM module with no imports. This interface requires JavaScript in the browser.
- PCM copies use persistent WASM views. STFT state, overlap-add buffers, and the 128-sample callback are preallocated; processing allocates no heap memory.
- 1024-point FFT, 256-sample hop, actual AudioContext rate. Clean/bypass and bitcrusher skip vocal FFT processing. Wet/dry and bypass use matched overlap-add timing with unity reconstruction.
- Analysis runs only for the selected display, on request. Eco requests 20 snapshots/sec; normal requests 40. At most one request is outstanding. Frozen, paused, hidden, and powered-off displays request no analysis.
- Fixed-resolution canvas and a bounded 600×256 scrolling spectrogram avoid growing histories, per-frame image allocation, and retina-driven rendering costs. Pitch history holds 240 readings.
- Audio files are capped at 40 MB / 10 minutes; MIDI at 2 MB. MIDI is parsed and indexed on load, not per hop. Loading MIDI may briefly interrupt audio while the worklet installs its timeline.
- No image assets, remote fonts, telemetry, or runtime dependencies. Both WASM modules and their generated glue total less than 500 KB uncompressed in the initial build.

## Scope and limitations

This is a shared-library DSP laboratory, not Virtual Daisy's ARM firmware emulator. It does not yet connect directly to Virtual Synthphone-E or Keyboard Keyboard. Effects use the pinned DSP library; that does not establish hardware or perceptual parity.

Modes: clean, pitch correction, saw-carrier vocoder, bitcrusher, up to eight MIDI harmony voices, and pitch/formant shift. Correction supports all 24 major/minor keys. MIDI tempo maps, note-off/velocity-zero events, and channel-specific note state are resolved; percussion channel 10 is excluded. MIDI format 2 is rejected, sustain pedal is not implemented. MIDI and audio loop independently. Seek restarts DSP at the corresponding MIDI time; held pitch/carrier targets reset when restarting.

The original desktop cepstrum, score view, ideal formant-envelope overlay, multi-size FFT, and MIDI track selector are not included in this first web edition. Envelope view plots measured cepstrally smoothed spectral envelopes, not a cepstrum. Spectrogram time depends on display cadence; pitch history is 240 readings (12 s in Eco, 6 s normally).

Microphone requires HTTPS or localhost and a browser with AudioWorklet support. It turns off on power-off, page exit, or source change. Browser decoding handles supported audio formats and converts file playback to the context rate. Input is downmixed to mono; output is mono. Listen uses 50% output gain for headroom; the analyser measures the signal before this monitor gain. The STFT adapter contributes 1024 samples of delay (~21.3 ms at 48 kHz), in addition to browser/device buffering. Waveform voltage divisions are normalized full-scale units, not calibrated physical volts.

## Verification

Rust tests cover unity-gain reconstruction, finite/bounded output in each mode, MIDI tempo-map timing and note-off, and mapping every displayed key to the DSP library's circle-of-fifths ordering. Browser QA exercises the live oscillator, effects, graph selection, freeze, and transport. These checks do not replace microphone/headphone listening or testing on a physical Synthphone.

AI made · [0rwx X](https://0rwx.com/).
