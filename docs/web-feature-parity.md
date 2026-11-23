# DSP Viewer web feature audit

Compared against the original desktop implementation in `src/main.rs`, `src/audio.rs`, `src/realtime.rs` and `src/midi.rs`, including the local desktop work adding real eight-note chords, MIDI pitch lock and three spectral envelopes. Desktop source and unrelated local edits were preserved. This audit concerns feature availability; microphone listening and physical hardware equivalence are not established.

| Original feature | Web implementation |
| --- | --- |
| Pitch correction | Shared Rust PitchControl; 24 major/minor keys, correction strength, retune speed and transpose |
| Vocoder | Shared Rust Vocode; 50–2,000 Hz manual carrier, sine/saw/square/triangle |
| MIDI vocoder | Highest active MIDI note controls carrier; manual fallback before first note, held target across rests |
| MIDI harmony | Up to eight distinct simultaneous notes, including chords within one track; separate strength and transpose |
| MIDI pitch lock | Highest active MIDI target; unshifted before first note; holds across rests; full correction to the MIDI target |
| Dry/formant | Uncorrected source with global formant/transpose controls |
| Pitch and harmony transpose | Separate −12 to +24 semitone controls, 0.5-semitone steps |
| Global formant shift | −24 to +24 semitones; original, intended and measured envelope curves |
| Global bit depth | 1–16 bits before the selected vocal effect |
| Global rate reduction | ÷1–32 before the selected vocal effect |
| One-second bypass | Timed comparison with countdown; another press extends it; matched dry-path delay |
| File input | Browser-supported local audio decoding; looping playback, play/pause, stop/rewind and seek |
| Microphone | Browser capture, initially muted monitoring, track cleanup on exit/power/source change |
| MIDI file input | SMF 0/1, tempo maps/SMPTE, track/channel-specific note-offs and eight-note chord state |
| Waveform | Actual emitted input-aligned/output samples; transient-preserving min/max reduction |
| Spectrum | Input/output FFT magnitudes with three independently switchable envelopes and harmonic markers |
| Spectrogram | Two incremental time/frequency histories, real cadence-derived duration |
| Pitch history | YIN from actual aligned input/output, note names and held MIDI target |
| MIDI staff | Treble/bass staves, pitches, accidentals, ledger lines, durations, track-dependent stem directions and playhead |
| Simultaneous analysis | Default Compare has paired waveforms, spectra, spectrograms and pitch; three additional selectable linked screens |
| Processed-block statistics | Measured engine block counts and explicitly estimated FFT/DFT operation comparisons |

Added scope controls: independent channel visibility, AC/DC/GND, sensitivity/position, concentric fine adjustment, timebase, trigger source/level/slope, Auto/Normal/Single, Hold, brightness/focus/grid/persistence. Added real cepstrum, X–Y, wet/dry blend, clean and isolated bitcrush references, oscillator input and short hover/focus tips.

The original running desktop did **not** expose an FFT-size selector, MIDI-track selector, pre/post gain or cepstrum screen; those appeared in earlier planning text. The web FFT remains 1024 with a 256-sample hop. An actual cepstrum is now an additional web feature.

## Verification

Fifteen Rust checks cover unity overlap-add reconstruction; actual finite/clamped output in eight modes; parameter/carrier ranges; global bitcrush and bypass; ideal envelope identity/extrapolation; pitch lock passing before its first note, reaching 440 Hz from 220 Hz, holding across a rest, and resetting at seek/audio-loop boundaries; MIDI tempo maps, channel independence, velocity-zero offs and eight simultaneous notes; prepared timeline resampling; key mapping; trigger edges; and transient-preserving plotting.

Browser checks use generated 16-second audio and 21 MIDI notes, including an eight-note chord. Verified local decoding, MIDI load/score, all eight effects and nine main views, four carrier selections, keyboard/fine controls, concise tips, linked screen selection, Hold, timed bypass expiry, Normal waiting, Single capture, pause/seek/resume/stop and power-off. Responsive layout was inspected at 390 px with no horizontal overflow. No console warnings/errors were observed during the exercised flows. These are signal-path and UI checks, not headphone listening or physical microphone/hardware validation.

## Explicit differences and bounds

- The shared DSP library revision uses streaming voice state. Native control parity does not guarantee identical sound to an older desktop/library combination. Harmony strength interpolates MIDI targets toward detected source pitch.
- Browser audio processing is mono and adds 1024 samples of adapter delay, plus browser/device buffering. Display channel coupling/scale does not alter sound. FS/div is normalized audio full scale, never calibrated volts.
- MIDI loops at its own duration. Audio-file looping or seeking also repositions MIDI; held targets reset on file-loop boundaries. Sustain pedal remains unsupported; format 2 is rejected explicitly. Channel 10 is available, as in the original native viewer.
- Histories are bounded at 512 readings/columns, MIDI score drawing at 2,000 visible notes. Audio-file limit is 512 MB; MIDI limit is 8 MB. Long-duration notes crossing score windows remain visible.
- File decoding and compact timeline installation can briefly interrupt playback. FFT counts/operation comparisons are estimates; block counts are snapshots of processed engine frames, accumulated across resets from the last received measurement.
- The photographic-style assets are generated project assets. This is a working web instrument, not a photograph of a real SynthPhone-branded hardware product, an electrical test instrument or the Virtual Daisy firmware emulator.
