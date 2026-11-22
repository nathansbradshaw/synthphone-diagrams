use synthphone_e_vocal_dsp::{
    MusicalSettings, ProcessingMode, VocalEffectsConfig, VocalStreamState,
    dsp::{Fft1024, FftOps, bitcrush, extract_cepstral_envelope, find_pitch_yin},
    process_vocal_effects_1024_stream,
};
const N: usize = 1024;
const HOP: usize = 256;
/// Preallocated PCM and STFT state. No heap allocation in `process`.
pub struct Engine {
    config: VocalEffectsConfig,
    settings: MusicalSettings,
    state: VocalStreamState,
    input: [f32; N],
    crushed: [f32; N],
    raw: [f32; N],
    output: [f32; N],
    ola: [f32; N],
    phases_in: [f32; N],
    phases_out: [f32; N],
    envelope: [f32; N / 2],
    inverse: [f32; N / 2],
    block: [f32; 128],
    snapshot: [f32; 3076],
    at: usize,
    mode: u8,
    bits: i8,
    reduction: usize,
    held: f32,
    hold_at: usize,
    phase: f32,
    frequency: f32,
    mix: f32,
    bypass: bool,
    midi: Option<crate::midi::Timeline>,
    clock: usize,
    last_target: f32,
}
impl Engine {
    pub fn new(sample_rate: f32) -> Engine {
        Self {
            config: VocalEffectsConfig {
                sample_rate,
                ..Default::default()
            },
            settings: MusicalSettings {
                mode: ProcessingMode::Dry,
                ..Default::default()
            },
            state: VocalStreamState::new(),
            input: [0.; N],
            crushed: [0.; N],
            raw: [0.; N],
            output: [0.; N],
            ola: [0.; N],
            phases_in: [0.; N],
            phases_out: [0.; N],
            envelope: [0.; N / 2],
            inverse: [0.; N / 2],
            block: [0.; 128],
            snapshot: [0.; 3076],
            at: 0,
            mode: 0,
            bits: 8,
            reduction: 1,
            held: 0.,
            hold_at: 0,
            phase: 0.,
            frequency: 220.,
            mix: 1.,
            bypass: false,
            midi: None,
            clock: 0,
            last_target: 0.,
        }
    }
    pub fn configure(&mut self, values: &[f32]) {
        if values.len() < 11 || values.iter().any(|v| !v.is_finite()) {
            return;
        }
        let mode = values[0] as u8;
        if mode != self.mode {
            self.state.reset();
            self.mode = mode;
        }
        self.settings.mode = match mode {
            1 => ProcessingMode::PitchControl,
            2 => ProcessingMode::Vocode,
            4 => ProcessingMode::Harmony,
            _ => ProcessingMode::Dry,
        };
        self.config.pitch_correction_strength = values[1].clamp(0., 1.);
        self.settings.octave_ratio = 2.0_f32.powf(values[2].clamp(-12., 12.) / 12.);
        let formant = values[3].clamp(0.5, 2.);
        self.settings.formant = if formant < 0.999 {
            1
        } else if formant > 1.001 {
            2
        } else {
            0
        };
        self.settings.formant_male_ratio = formant;
        self.settings.formant_female_ratio = formant;
        self.bits = values[4].clamp(1., 16.) as i8;
        self.reduction = values[5].clamp(1., 32.) as usize;
        self.frequency = values[6].clamp(55., 880.);
        self.mix = values[7].clamp(0., 1.);
        self.bypass = values[8] > 0.;
        self.settings.key = values[9].clamp(0., 23.) as i8;
        self.config.transition_speed = values[10].clamp(0.01, 1.);
    }
    pub fn load_midi(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.midi = Some(crate::midi::Timeline::parse(
            bytes,
            self.config.sample_rate,
        )?);
        self.clock = 0;
        self.last_target = 0.;
        Ok(())
    }
    /// Pointers let the JS-only browser callback copy PCM without per-quantum Vecs.
    pub fn input_ptr(&mut self) -> *mut f32 {
        self.block.as_mut_ptr()
    }
    pub fn seek(&mut self, seconds: f64) {
        self.clock = (seconds.max(0.) * self.config.sample_rate as f64) as usize;
    }
    pub fn output_ptr(&self) -> *const f32 {
        self.block.as_ptr()
    }
    pub fn snapshot_ptr(&self) -> *const f32 {
        self.snapshot.as_ptr()
    }
    pub fn process(&mut self) {
        for i in 0..128 {
            let x = if self.block[i].is_finite() {
                self.block[i]
            } else {
                0.
            };
            self.input[N - HOP + self.at] = x;
            if self.hold_at == 0 {
                self.held = bitcrush(x, self.bits);
            }
            self.crushed[N - HOP + self.at] = self.held;
            self.hold_at = (self.hold_at + 1) % self.reduction;
            self.block[i] = self.ola[self.at].clamp(-1., 1.);
            self.at += 1;
            if self.at == HOP {
                self.at = 0;
                self.ola.copy_within(HOP.., 0);
                self.ola[N - HOP..].fill(0.);
                self.frame();
                self.input.copy_within(HOP.., 0);
                self.crushed.copy_within(HOP.., 0);
                self.clock += HOP;
            }
        }
    }
    /// Expensive analysis runs only on a display request, never every STFT hop.
    pub fn analyze(&mut self, view: u8) {
        self.snapshot[..N].copy_from_slice(&self.raw);
        self.snapshot[N..2 * N].copy_from_slice(&self.output);
        self.snapshot[3072] = 0.;
        self.snapshot[3073] = 0.;
        if view == 3 {
            self.snapshot[3072] = find_pitch_yin(&self.raw, &self.config);
            self.snapshot[3073] = find_pitch_yin(&self.output, &self.config);
        }
        if view == 1 || view == 2 || view == 4 {
            let mut input = self.raw;
            let mut output = self.output;
            for i in 0..N {
                input[i] *= Fft1024::get_hann_window()[i];
                output[i] *= Fft1024::get_hann_window()[i];
            }
            let mut mags_in = [0.; N / 2];
            let mut mags_out = [0.; N / 2];
            for (slot, c) in mags_in.iter_mut().zip(Fft1024::forward_fft(&mut input)) {
                *slot = (c.re * c.re + c.im * c.im).sqrt();
            }
            for (slot, c) in mags_out.iter_mut().zip(Fft1024::forward_fft(&mut output)) {
                *slot = (c.re * c.re + c.im * c.im).sqrt();
            }
            // Packed rfft bin zero stores DC + Nyquist, so exclude Nyquist.
            mags_in[0] = input[0].abs();
            mags_out[0] = output[0].abs();
            if view == 4 {
                let mut a = [0.; N / 2];
                let mut b = [0.; N / 2];
                extract_cepstral_envelope::<N, { N / 2 }, Fft1024>(&mags_in, &mut a);
                extract_cepstral_envelope::<N, { N / 2 }, Fft1024>(&mags_out, &mut b);
                mags_in = a;
                mags_out = b;
            }
            self.snapshot[2048..2560].copy_from_slice(&mags_in);
            self.snapshot[2560..3072].copy_from_slice(&mags_out);
        }
        self.snapshot[3074] = self.raw.iter().map(|v| v * v).sum::<f32>().sqrt() / 32.;
        self.snapshot[3075] = self.output.iter().map(|v| v * v).sum::<f32>().sqrt() / 32.;
    }
}
impl Engine {
    fn frame(&mut self) {
        self.raw = self.input;
        if let Some(midi) = &self.midi {
            self.settings.midi_frequencies = midi.frequencies_at(self.clock);
            if self.settings.midi_frequencies[0] > 0. {
                self.last_target = self.settings.midi_frequencies[0];
            }
            if self.mode == 1 {
                self.settings.midi_frequencies[0] = self.last_target;
            }
        }
        let direct = self.bypass || self.mode == 0 || self.mode == 3;
        let mut block = self.input;
        if self.mode == 3 && !self.bypass {
            block = self.crushed;
        }
        if direct {
            let mix = if self.bypass { 0. } else { self.mix };
            for i in 0..N {
                self.ola[i] += (block[i] * mix + self.input[i] * (1. - mix))
                    * Fft1024::get_hann_window()[i].powi(2)
                    * (2. / 3.);
            }
        } else {
            let mut carrier = [0.; N];
            let frequency = if self.mode == 2 && self.last_target > 0. {
                self.last_target
            } else {
                self.frequency
            };
            let phase = self.phase;
            for (i, sample) in carrier.iter_mut().enumerate() {
                *sample = 0.3
                    * (2. * ((phase + i as f32 * frequency / self.config.sample_rate).fract())
                        - 1.);
            }
            self.phase = (phase + HOP as f32 * frequency / self.config.sample_rate).fract();
            let out = process_vocal_effects_1024_stream(
                &mut block,
                Some(&mut carrier),
                &mut self.phases_in,
                &mut self.phases_out,
                &mut self.envelope,
                &mut self.inverse,
                &mut self.state,
                &self.config,
                &self.settings,
            );
            for (i, wet) in out.iter().enumerate() {
                let dry = self.input[i] * Fft1024::get_hann_window()[i].powi(2);
                self.ola[i] += (wet * self.mix + dry * (1. - self.mix)) * (2. / 3.);
            }
        }
        // Plot the actual overlap-added audio that will leave the worklet.
        self.output.copy_within(HOP.., 0);
        self.output[N - HOP..].copy_from_slice(&self.ola[..HOP]);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bypass_reconstructs_unity_after_startup() {
        let mut dsp = Engine::new(48000.);
        for block in 0..80 {
            for i in 0..128 {
                dsp.block[i] = ((block * 128 + i) as f32 * 0.03).sin() * 0.3;
            }
            let expected: [f32; 128] =
                std::array::from_fn(|i| (((block * 128 + i) as f32 - 1024.) * 0.03).sin() * 0.3);
            dsp.process();
            if block > 12 {
                for (a, b) in dsp.block.iter().zip(expected) {
                    assert!((a - b).abs() < 0.002, "{a} {b}");
                }
            }
        }
    }
    #[test]
    fn all_modes_are_finite_and_analysis_has_signal() {
        for mode in 0..=5 {
            let mut dsp = Engine::new(48000.);
            dsp.configure(&[mode as f32, 0.8, 0., 1., 5., 3., 220., 1., 0., 0., 0.15]);
            for block in 0..150 {
                for i in 0..128 {
                    dsp.block[i] =
                        ((block * 128 + i) as f32 * std::f32::consts::TAU * 220. / 48000.).sin()
                            * 0.3;
                }
                dsp.process();
                assert!(dsp.block.iter().all(|v| v.is_finite() && v.abs() <= 1.));
            }
            dsp.analyze(1);
            assert!(dsp.snapshot[2048..2560].iter().any(|v| *v > 1.));
            if mode != 4 {
                assert!(dsp.output.iter().any(|v| v.abs() > 0.001), "mode {mode}");
            }
        }
    }
}
