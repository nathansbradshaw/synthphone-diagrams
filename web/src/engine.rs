use synthphone_e_vocal_dsp::{
    MusicalSettings, ProcessingMode, VocalEffectsConfig, VocalStreamState,
    dsp::{Fft1024, FftOps, bitcrush, extract_cepstral_envelope, find_pitch_yin},
    process_vocal_effects_1024_stream,
};
pub const N: usize = 1024;
pub const HOP: usize = 256;
pub const WAVE: usize = 2048;
pub const FFT_A: usize = 2 * WAVE;
pub const FFT_B: usize = FFT_A + 512;
pub const ENV_A: usize = FFT_B + 512;
pub const ENV_IDEAL: usize = ENV_A + 512;
pub const ENV_B: usize = ENV_IDEAL + 512;
pub const CEP_A: usize = ENV_B + 512;
pub const CEP_B: usize = CEP_A + 512;
pub const META: usize = CEP_B + 512;
pub const SNAPSHOT_LEN: usize = META + 16;
pub const PARAM_COUNT: usize = 14;
/// Fixed-size state for the browser's 128-sample quantum. Analysis is demand-driven.
pub struct Engine {
    wave_a: [f32; WAVE],
    wave_b: [f32; WAVE],
    config: VocalEffectsConfig,
    settings: MusicalSettings,
    state: VocalStreamState,
    input: [f32; N],
    crushed: [f32; N],
    raw: [f32; N],
    output: [f32; N],
    ola: [f32; N],
    dry: [f32; N],
    phases_in: [f32; N],
    phases_out: [f32; N],
    envelope: [f32; N / 2],
    inverse: [f32; N / 2],
    average_a: [f32; N / 2],
    average_b: [f32; N / 2],
    block: [f32; 128],
    snapshot: [f32; SNAPSHOT_LEN],
    at: usize,
    mode: u8,
    bits: i8,
    reduction: usize,
    held: f32,
    hold_at: usize,
    phase: f32,
    frequency: f32,
    wave: u8,
    mix: f32,
    current_mix: f32,
    bypass: bool,
    midi: Option<crate::midi::Timeline>,
    clock: usize,
    audio_cycle: usize,
    last_target: f32,
    last_position: usize,
    frames: u64,
    fft_calls: u64,
    makeup: f32,
    harmony_strength: f32,
    harmony_source: f32,
}
impl Engine {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            wave_a: [0.; WAVE],
            wave_b: [0.; WAVE],
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
            dry: [0.; N],
            phases_in: [0.; N],
            phases_out: [0.; N],
            envelope: [0.; N / 2],
            inverse: [0.; N / 2],
            average_a: [0.; N / 2],
            average_b: [0.; N / 2],
            block: [0.; 128],
            snapshot: [0.; SNAPSHOT_LEN],
            at: 0,
            mode: 0,
            bits: 16,
            reduction: 1,
            held: 0.,
            hold_at: 0,
            phase: 0.,
            frequency: 220.,
            wave: 1,
            mix: 1.,
            current_mix: 1.,
            bypass: false,
            midi: None,
            clock: 0,
            audio_cycle: 0,
            last_target: 0.,
            last_position: 0,
            frames: 0,
            fft_calls: 0,
            makeup: 1.,
            harmony_strength: 0.8,
            harmony_source: 0.,
        }
    }
    pub fn configure(&mut self, v: &[f32]) {
        if v.len() < PARAM_COUNT || v.iter().any(|v| !v.is_finite()) {
            return;
        }
        let mode = v[0] as u8;
        if mode != self.mode {
            self.state.reset();
            self.mode = mode;
            self.phases_in.fill(0.);
            self.phases_out.fill(0.);
            self.last_target = 0.;
            self.makeup = 1.;
        }
        self.settings.mode = match mode {
            1 | 7 => ProcessingMode::PitchControl,
            2 | 6 => ProcessingMode::Vocode,
            4 => ProcessingMode::Harmony,
            _ => ProcessingMode::Dry,
        };
        self.config.pitch_correction_strength = if mode == 7 { 1. } else { v[1].clamp(0., 1.) };
        let shift = if mode == 4 { v[13] } else { v[2] };
        self.settings.octave_ratio = 2f32.powf(shift.clamp(-12., 24.) / 12.);
        let formant = v[3].clamp(-24., 24.);
        self.settings.formant = if formant < -0.05 {
            1
        } else if formant > 0.05 {
            2
        } else {
            0
        };
        self.settings.formant_male_ratio = 2f32.powf(formant / 12.);
        self.settings.formant_female_ratio = self.settings.formant_male_ratio;
        self.bits = v[4].clamp(1., 16.).round() as i8;
        self.reduction = v[5].clamp(1., 32.).round() as usize;
        self.hold_at %= self.reduction;
        self.frequency = v[6].clamp(50., 2000.);
        self.mix = v[7].clamp(0., 1.);
        self.bypass = v[8] > 0.;
        self.settings.key = v[9].clamp(0., 23.) as i8;
        self.config.transition_speed = v[10].clamp(0.01, 1.);
        self.wave = v[11].clamp(0., 3.) as u8;
        self.harmony_strength = v[12].clamp(0., 1.);
    }
    pub fn load_midi(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.midi = Some(crate::midi::Timeline::parse(
            bytes,
            self.config.sample_rate,
        )?);
        self.last_target = 0.;
        Ok(())
    }
    pub fn input_ptr(&mut self) -> *mut f32 {
        self.block.as_mut_ptr()
    }
    pub fn output_ptr(&self) -> *const f32 {
        self.block.as_ptr()
    }
    pub fn snapshot_ptr(&self) -> *const f32 {
        self.snapshot.as_ptr()
    }
    pub fn seek(&mut self, seconds: f64) {
        self.clock = (seconds.max(0.) * self.config.sample_rate as f64) as usize;
        self.last_target = 0.;
        self.last_position = self.position();
        self.state.reset();
    }
    pub fn audio_duration(&mut self, seconds: f64) {
        self.audio_cycle = (seconds.max(0.) * self.config.sample_rate as f64) as usize;
    }
    pub fn process(&mut self) {
        self.wave_a.copy_within(128.., 0);
        self.wave_b.copy_within(128.., 0);
        self.output.copy_within(128.., 0);
        self.raw.copy_within(128.., 0);
        for i in 0..128 {
            let x = if self.block[i].is_finite() {
                self.block[i]
            } else {
                0.
            };
            self.input[N - HOP + self.at] = x;
            if self.hold_at == 0 {
                self.held = if self.bits == 16 {
                    x
                } else {
                    bitcrush(x, self.bits)
                };
            }
            self.crushed[N - HOP + self.at] = self.held;
            self.hold_at = (self.hold_at + 1) % self.reduction;
            let target = if self.bypass { 0. } else { self.mix };
            self.current_mix += (target - self.current_mix) * 0.025;
            let y = (self.ola[self.at] * self.current_mix
                + self.dry[self.at] * (1. - self.current_mix))
                .clamp(-1., 1.);
            self.block[i] = y;
            self.output[N - 128 + i] = y;
            self.raw[N - 128 + i] = self.dry[self.at];
            self.wave_a[WAVE - 128 + i] = self.raw[N - 128 + i];
            self.wave_b[WAVE - 128 + i] = y;
            self.at += 1;
            if self.at == HOP {
                self.at = 0;
                self.ola.copy_within(HOP.., 0);
                self.ola[N - HOP..].fill(0.);
                self.dry.copy_within(HOP.., 0);
                self.dry[N - HOP..].fill(0.);
                self.frame();
                self.input.copy_within(HOP.., 0);
                self.crushed.copy_within(HOP.., 0);
                self.clock += HOP;
            }
        }
    }
    /// A bit mask lets Compare request only the analyses it actually displays.
    pub fn analyze(&mut self, mask: u8) {
        self.snapshot[..WAVE].copy_from_slice(&self.wave_a);
        self.snapshot[WAVE..2 * WAVE].copy_from_slice(&self.wave_b);
        if mask & 4 != 0 {
            self.snapshot[META] = find_pitch_yin(&self.raw, &self.config);
            self.snapshot[META + 1] = find_pitch_yin(&self.output, &self.config);
        }
        if mask & 1 != 0 || mask & 2 != 0 || mask & 8 != 0 {
            let mut a = self.raw;
            let mut b = self.output;
            for i in 0..N {
                a[i] *= Fft1024::get_hann_window()[i];
                b[i] *= Fft1024::get_hann_window()[i];
            }
            let mut ma = [0.; N / 2];
            let mut mb = [0.; N / 2];
            for (m, c) in ma.iter_mut().zip(Fft1024::forward_fft(&mut a)) {
                *m = (c.re * c.re + c.im * c.im).sqrt();
            }
            for (m, c) in mb.iter_mut().zip(Fft1024::forward_fft(&mut b)) {
                *m = (c.re * c.re + c.im * c.im).sqrt();
            }
            ma[0] = a[0].abs();
            mb[0] = b[0].abs();
            self.fft_calls += 2;
            self.snapshot[FFT_A..FFT_B].copy_from_slice(&ma);
            self.snapshot[FFT_B..ENV_A].copy_from_slice(&mb);
            if mask & 2 != 0 {
                for i in 0..N / 2 {
                    self.average_a[i] += (ma[i] - self.average_a[i]) * 0.2;
                    self.average_b[i] += (mb[i] - self.average_b[i]) * 0.2;
                }
                let mut ea = [0.; N / 2];
                let mut eb = [0.; N / 2];
                extract_cepstral_envelope::<N, { N / 2 }, Fft1024>(&self.average_a, &mut ea);
                extract_cepstral_envelope::<N, { N / 2 }, Fft1024>(&self.average_b, &mut eb);
                self.fft_calls += 4;
                let ratio = if self.settings.formant == 0 {
                    1.
                } else {
                    self.settings.formant_male_ratio
                };
                self.snapshot[ENV_A..ENV_IDEAL].copy_from_slice(&ea);
                self.snapshot[ENV_B..CEP_A].copy_from_slice(&eb);
                for i in 0..N / 2 {
                    self.snapshot[ENV_IDEAL + i] = sample_envelope(&ea, i as f32 / ratio);
                }
            }
            if mask & 8 != 0 {
                for (m, offset) in [(&ma, CEP_A), (&mb, CEP_B)] {
                    let mut logarithm = [0.; N];
                    for i in 0..N / 2 {
                        let v = (m[i] / 256.).max(1e-8).ln();
                        logarithm[i] = v;
                        if i > 0 {
                            logarithm[N - i] = v;
                        }
                    }
                    logarithm[N / 2] = logarithm[N / 2 - 1];
                    for (i, c) in Fft1024::forward_fft(&mut logarithm).iter().enumerate() {
                        self.snapshot[offset + i] = c.re / N as f32;
                    }
                    self.fft_calls += 1;
                }
            }
        }
        self.snapshot[META + 2] = rms(&self.raw);
        self.snapshot[META + 3] = rms(&self.output);
        self.snapshot[META + 4] = peak(&self.raw);
        self.snapshot[META + 5] = peak(&self.output);
        self.snapshot[META + 6] = self.last_target;
        self.snapshot[META + 7] = self.frames as f32;
        self.snapshot[META + 8] = self.fft_calls as f32;
        self.snapshot[META + 9] = self.config.sample_rate;
        self.snapshot[META + 10] = self.position() as f32 / self.config.sample_rate;
        self.snapshot[META + 11] = self
            .midi
            .as_ref()
            .map(|m| m.duration() as f32)
            .unwrap_or(0.);
        self.snapshot[META + 12] = if self.bypass { 1. } else { 0. };
        self.snapshot[META + 13] = self.mode as f32;
        self.snapshot[META + 14] = mask as f32;
    }
    fn position(&self) -> usize {
        if self.audio_cycle > 0 {
            self.clock % self.audio_cycle
        } else {
            self.clock
        }
    }
    fn frame(&mut self) {
        self.frames += 1;
        let position = self.position();
        if position < self.last_position {
            self.last_target = 0.;
        }
        self.last_position = position;
        let mut settings = self.settings;
        if let Some(midi) = &self.midi {
            let (chord, target) = midi.state_at(self.position());
            settings.midi_frequencies = if self.mode == 4 { chord } else { [0.; 8] };
            if target > 0. {
                self.last_target = target;
            }
        }
        if self.mode == 7 {
            if self.last_target > 0. {
                settings.midi_frequencies[0] = self.last_target;
            } else {
                settings.mode = ProcessingMode::Dry;
                settings.octave_ratio = 1.;
            }
        }
        if self.mode == 4 && self.harmony_strength < 0.999 {
            if self.frames % 8 == 1 || self.harmony_source == 0. {
                self.harmony_source = find_pitch_yin(&self.input, &self.config);
            }
            if self.harmony_source > 0. {
                for f in &mut settings.midi_frequencies {
                    if *f > 0. {
                        *f = self.harmony_source
                            * (*f / self.harmony_source).powf(self.harmony_strength);
                    }
                }
            }
        }
        let mut block = self.crushed;
        let direct = self.mode == 0 || self.mode == 3;
        let result = if direct {
            let source = if self.mode == 0 {
                &self.input
            } else {
                &self.crushed
            };
            std::array::from_fn(|i| source[i] * Fft1024::get_hann_window()[i].powi(2))
        } else {
            let f = if self.mode == 6 && self.last_target > 0. {
                self.last_target
            } else {
                self.frequency
            };
            let mut carrier = [0.; N];
            let phase = self.phase;
            for (i, x) in carrier.iter_mut().enumerate() {
                let p = (phase + i as f32 * f / self.config.sample_rate).fract();
                *x = 0.3 * carrier_sample(self.wave, p);
            }
            self.phase = (phase + HOP as f32 * f / self.config.sample_rate).fract();
            self.fft_calls += if matches!(self.mode, 2 | 6) { 3 } else { 2 };
            process_vocal_effects_1024_stream(
                &mut block,
                Some(&mut carrier),
                &mut self.phases_in,
                &mut self.phases_out,
                &mut self.envelope,
                &mut self.inverse,
                &mut self.state,
                &self.config,
                &settings,
            )
        };
        for i in 0..N {
            self.ola[i] += result[i] * (2. / 3.);
            self.dry[i] += self.input[i] * Fft1024::get_hann_window()[i].powi(2) * (2. / 3.);
        }
        if matches!(self.mode, 2 | 6) {
            let ie = self.input[..HOP].iter().map(|x| x * x).sum::<f32>();
            let oe = self.ola[..HOP].iter().map(|x| x * x).sum::<f32>();
            if ie > 1e-8 && oe > 1e-8 {
                let target = (ie / oe).sqrt().clamp(0.25, 8.);
                self.makeup += (target - self.makeup) * 0.15;
                for s in &mut self.ola[..HOP] {
                    *s *= self.makeup;
                }
            }
        }
    }
}
fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
}
fn peak(x: &[f32]) -> f32 {
    x.iter().map(|v| v.abs()).fold(0., f32::max)
}
pub fn carrier_sample(wave: u8, phase: f32) -> f32 {
    match wave {
        0 => (phase * std::f32::consts::TAU).sin(),
        2 => {
            if phase < 0.5 {
                1.
            } else {
                -1.
            }
        }
        3 => 1. - 4. * (phase - 0.5).abs(),
        _ => 2. * phase - 1.,
    }
}
/// Same bounded extrapolation as the native viewer and shared DSP library.
fn sample_envelope(e: &[f32; N / 2], position: f32) -> f32 {
    let last = N / 2 - 1;
    let p = position.max(0.);
    if p <= last as f32 {
        let i = p as usize;
        let fraction = p - i as f32;
        if i < last {
            e[i] * (1. - fraction) + e[i + 1] * fraction
        } else {
            e[i]
        }
    } else {
        let end = e[last].max(1e-6);
        let previous = e[last - 1].max(1e-6);
        end * (end / previous).clamp(0.05, 1.).powf(p - last as f32)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn parameters(mode: u8) -> [f32; PARAM_COUNT] {
        [
            mode as f32,
            0.8,
            0.,
            0.,
            16.,
            1.,
            220.,
            1.,
            0.,
            0.,
            0.15,
            1.,
            0.8,
            0.,
        ]
    }
    fn feed(d: &mut Engine, blocks: usize) {
        for block in 0..blocks {
            for i in 0..128 {
                d.block[i] =
                    ((block * 128 + i) as f32 * std::f32::consts::TAU * 220. / 48000.).sin() * 0.3;
            }
            d.process();
            assert!(d.block.iter().all(|x| x.is_finite() && x.abs() <= 1.));
        }
    }
    #[test]
    fn clean_reconstruction_unity() {
        let mut d = Engine::new(48000.);
        for block in 0..80 {
            for i in 0..128 {
                d.block[i] = ((block * 128 + i) as f32 * 0.03).sin() * 0.3;
            }
            d.process();
            if block > 12 {
                for i in 0..128 {
                    let expected = (((block * 128 + i) as f32 - 1024.) * 0.03).sin() * 0.3;
                    assert!((d.block[i] - expected).abs() < 0.002);
                }
            }
        }
    }
    #[test]
    fn all_modes_finite_and_real_output_snapshot() {
        for mode in 0..8 {
            let mut d = Engine::new(48000.);
            d.configure(&parameters(mode));
            feed(&mut d, 120);
            d.analyze(15);
            assert!(d.snapshot[FFT_A..FFT_B].iter().any(|x| *x > 1.));
            assert_eq!(&d.snapshot[WAVE..2 * WAVE], &d.wave_b);
            assert!(d.snapshot.iter().all(|x| x.is_finite()));
        }
    }
    #[test]
    fn global_crushing_operates_in_dry_and_bypass_returns_clean() {
        let mut d = Engine::new(48000.);
        let mut p = parameters(5);
        p[4] = 3.;
        p[5] = 7.;
        d.configure(&p);
        feed(&mut d, 100);
        let crushed = d.output;
        p[8] = 1.;
        d.configure(&p);
        feed(&mut d, 100);
        assert!(
            crushed
                .iter()
                .zip(d.output)
                .map(|(a, b)| (a - b).abs())
                .sum::<f32>()
                > 1.
        );
    }
    #[test]
    fn native_ranges_and_carrier_waves() {
        let mut d = Engine::new(48000.);
        let mut p = parameters(5);
        p[2] = 24.;
        p[3] = -24.;
        p[6] = 2000.;
        d.configure(&p);
        assert_eq!(d.settings.octave_ratio, 4.);
        assert_eq!(d.settings.formant_male_ratio, 0.25);
        assert_eq!(d.frequency, 2000.);
        assert!((carrier_sample(0, 0.25) - 1.).abs() < 1e-6);
        assert_eq!(carrier_sample(1, 0.25), -0.5);
        assert_eq!(carrier_sample(2, 0.25), 1.);
        assert_eq!(carrier_sample(3, 0.25), 0.);
    }
    #[test]
    fn ideal_envelope_is_identity_at_unity_and_decay_bounded() {
        let mut d = Engine::new(48000.);
        feed(&mut d, 70);
        d.analyze(3);
        assert_eq!(&d.snapshot[ENV_A..ENV_IDEAL], &d.snapshot[ENV_IDEAL..ENV_B]);
        let e = std::array::from_fn(|i| 1. / (i + 1) as f32);
        assert!(sample_envelope(&e, 1024.) < e[511]);
    }
    fn target_midi() -> Vec<u8> {
        // A4 from 0.5 to 1 second, followed by one second of rest.
        let events = [
            0x83, 0x60, 0x90, 69, 100, 0x83, 0x60, 0x80, 69, 0, 0x87, 0x40, 0xff, 0x2f, 0,
        ];
        let mut bytes = b"MThd\0\0\0\x06\0\0\0\x01\x01\xe0MTrk".to_vec();
        bytes.extend((events.len() as u32).to_be_bytes());
        bytes.extend(events);
        bytes
    }
    #[test]
    fn pitch_lock_passes_before_target_locks_and_holds_across_rest() {
        let mut d = Engine::new(48000.);
        d.configure(&parameters(7));
        d.load_midi(&target_midi()).unwrap();
        feed(&mut d, 120);
        d.analyze(4);
        assert_eq!(d.last_target, 0.);
        assert!(
            (d.snapshot[META + 1] - 220.).abs() < 4.,
            "{}",
            d.snapshot[META + 1]
        );
        d.seek(0.55);
        feed(&mut d, 140);
        d.analyze(4);
        assert_eq!(d.last_target, 440.);
        assert!(
            (d.snapshot[META + 1] - 440.).abs() < 8.,
            "{}",
            d.snapshot[META + 1]
        );
        d.seek(0.9);
        feed(&mut d, 140);
        assert_eq!(d.last_target, 440.);
        d.seek(0.);
        feed(&mut d, 100);
        assert_eq!(d.last_target, 0.);
    }
    #[test]
    fn audio_loop_releases_prior_midi_target_before_first_note() {
        let mut d = Engine::new(48000.);
        d.configure(&parameters(7));
        d.load_midi(&target_midi()).unwrap();
        d.audio_duration(1.5);
        d.seek(1.4);
        d.last_target = 440.;
        feed(&mut d, 80);
        assert_eq!(d.last_target, 0.);
    }
}
impl Engine {
    pub fn load_timeline(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.midi = Some(crate::midi::Timeline::decode(
            bytes,
            self.config.sample_rate,
        )?);
        self.last_target = 0.;
        Ok(())
    }
}
