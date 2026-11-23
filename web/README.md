# DSP Viewer · Web instrument

**[Open DSP Viewer](https://nathansbradshaw.github.io/synthphone-diagrams/)** · public GitHub Pages

1. **Power** starts the test oscillator. **Listen** enables sound.
2. **Compare** shows eight input/output measurements together.
3. Three additional screens sit in a separate unit at the top. Turn each **VIEW** selector to choose its measurement. **DISPLAYS** shows or hides the unit; **Hold** freezes all screens while audio continues.
4. Load local **audio** and **MIDI**, then choose a processor. Hover or focus a control for one short tip.

Drag knobs around their center or vertically. Arrow keys adjust them; Shift slows dragging; double-click resets. The smaller concentric **VAR** controls make fine scale adjustments. Coupling switches support clicking, dragging and arrow keys. Key and display selection use rotary controls; Major/Minor use buttons. Use headphones when listening to a microphone.

Audio and MIDI stay in your browser. No account, upload, telemetry or remote processing is involved. The desktop application remains in `src/`.

## Measurements and processing

All six original processors are available: pitch correction, vocoder, MIDI vocoder, eight-note MIDI harmony, MIDI pitch lock, and dry/formant shifting. Clean and isolated bitcrush modes are extra references. All four carrier waveforms, separate pitch/harmony strength and transpose, key/scale, global formant shift, bit depth, rate reduction and timed one-second bypass are retained. [Feature-by-feature audit](../docs/web-feature-parity.md).

Nine main views: waveform, spectrum, spectrogram, pitch history, formant envelopes, MIDI staff score, Compare, real cepstrum and X–Y. The auxiliary screens can each show spectrum, spectrogram, pitch, envelopes, MIDI, cepstrum or X–Y. They use the same snapshot and shared Hold state.

Channel sensitivity, position, AC/DC/GND, visibility, timebase and Auto/Normal/Single trigger provide oscilloscope controls. These operate on the display. **FS/div means normalized audio full scale**, not electrical volts. The three formant envelopes show original, intended shift, and actual measured output.

## Build and preview

Install Rust's `wasm32-unknown-unknown` target and `wasm-bindgen-cli` **0.2.127**. No Node, npm, Python or frontend framework is required.

From `web/`:

```sh
cargo run --manifest-path xtask/Cargo.toml -- build
cargo run --manifest-path xtask/Cargo.toml -- serve 3040
```

Open `http://127.0.0.1:3040`. Ctrl-C stops the loopback-only server. `dist/` is disposable and ignored. Set `CARGO_TARGET_DIR` to an existing absolute cache path to reuse it for the build tool and both WASM modules.

```sh
cargo test --locked
cargo fmt -- --check
cargo fmt --manifest-path xtask/Cargo.toml -- --check
cargo run --example fixtures -- /tmp/dsp-viewer-fixtures
```

The last command generates a 16-second WAV and MIDI with an eight-note chord for manual browser checks. It does not start a server or upload files. DSP is pinned to a published Git revision; no sibling checkout is required.

## GitHub Pages

Build locally, commit source to `main`, then copy `dist/` to the existing `gh-pages` branch and push. Preserve deployment history. GitHub Pages performs only final static publication; there is no cloud Rust build. `.nojekyll`, relative asset paths and versioned bootstrap/worklet URLs support the repository URL prefix. `version.json` identifies source commit, asset build hash and whether tracked source was dirty.

## Efficiency

- Rust owns controls, plotting, MIDI parsing, DSP and the build/server. A minimal browser-required JavaScript AudioWorklet bridge copies PCM to an import-free Rust WASM module.
- Audio processing uses preallocated fixed arrays and cached WASM views. The 128-sample callback does not allocate. STFT uses 1024 samples with a 256-sample hop.
- Audio remains continuous while displays update at 20 Hz in Eco or 40 Hz normally. Only analyses needed by visible screens run; at most one snapshot is outstanding. Hold, pause, hidden pages and power-off stop analysis requests. Hide the additional displays to avoid their extra work.
- Spectrograms scroll one column at a time in two 512×128 offscreen canvases. Pitch and timestamp histories cap at 512 readings. Canvas resolution is fixed independently of screen density.
- MIDI tempo maps, chord states and target notes are prepared on the UI thread, then installed in the audio module as a compact timeline. Score rendering caps visible notes at 2,000. No MIDI parsing occurs per audio hop.
- Local audio is limited to 512 MB and MIDI to 8 MB; there is no ten-minute cutoff. Decoded audio can use more memory than its compressed file. Loading or replacing a large timeline can briefly interrupt audio.
- Six optimized photographic-style component assets total about 440 KB. There are no remote fonts, image requests or runtime dependencies. [Asset provenance and prompts](assets/README.md).

## Limits and verification

This uses the shared Rust vocal DSP library. It is not Virtual Daisy's ARM firmware emulator and does not yet connect directly to Virtual Synthphone-E or Keyboard Keyboard. Preserving controls and processing paths does not establish hardware or perceptual equivalence.

MIDI formats 0/1, tempo maps, SMPTE timing, channel-specific notes, velocity-zero note-offs and up to eight simultaneous distinct pitches are supported. Channel 10 remains available as in the original viewer. Format 2 is rejected; sustain pedal is not implemented. MIDI loops at its own duration; an audio-file loop or seek also restarts/repositions its MIDI clock. Pitch lock and MIDI carrier hold the last played target across rests, resetting on restart, seek and audio-loop boundaries. Harmony strength interpolates targets toward the source pitch in log-frequency space.

Microphone needs HTTPS/localhost and AudioWorklet support; it starts muted and releases tracks on power-off, source change or page exit. File decoding uses browser formats and resamples to the context rate. Processing is mono. Listen has 50% gain for headroom; meters measure output before monitor gain. The adapter adds 1024 samples of delay (about 21.3 ms at 48 kHz), plus browser/device latency. FFT counts and DFT operation comparisons are estimates; no CPU percentage is claimed.

Rust verification covers unity reconstruction, bounded actual output, all eight modes, MIDI timing/chords, pitch lock before/during/after a note, audio-loop target reset, global pre-effects, parameter ranges, envelope extrapolation, transient-preserving plotting and trigger edges. Browser verification covers the live signal path, effects/views, local WAV/MIDI, linked screens, keyboard knobs/tips, Hold/trigger and transport. Physical microphone/headphone listening and physical Synthphone comparison remain unverified.

[Reference-photo critique](../docs/oscilloscope-design-review.md) · AI made · [0rwx X](https://0rwx.com/).
