/// Real-time DSP engine — file and microphone sources share the same pipeline.
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};

use synthphone_e_vocal_dsp::{
    ProcessingMode,
    audio::Oscillator,
    dsp::{
        Fft1024, FftOps, bitcrush, extract_cepstral_envelope, find_pitch_yin, sample_rate_reduce,
    },
    process_vocal_effects_1024,
};

use crate::audio::DspParams;
use crate::midi::MidiTimeline;
use crate::mic::MicRing;

pub const N: usize = 1024;
pub const HALF: usize = N / 2;

// ── Shared visualization data ────────────────────────────────────────────────

pub struct VisualizationSnapshot {
    pub input_window: Vec<f32>,
    pub output_window: Vec<f32>,
    pub input_fft: Vec<f32>,
    pub output_fft: Vec<f32>,
    pub formant_envelope: Vec<f32>,
    pub pitch_hz: f32,
    pub output_pitch_hz: f32,
    /// Latest input FFT column for the input spectrogram.
    pub input_spec_col: Vec<f32>,
    /// Latest output FFT column for the output spectrogram.
    pub output_spec_col: Vec<f32>,
    /// Increments every frame so the GUI can detect new columns.
    pub col_serial: u64,
    pub play_position: usize,
    pub total_samples: usize,
}

impl Default for VisualizationSnapshot {
    fn default() -> Self {
        Self {
            input_window: vec![0.0; N],
            output_window: vec![0.0; N],
            input_fft: vec![0.0; HALF],
            output_fft: vec![0.0; HALF],
            formant_envelope: vec![0.0; HALF],
            pitch_hz: 0.0,
            output_pitch_hz: 0.0,
            input_spec_col: vec![0.0; HALF],
            output_spec_col: vec![0.0; HALF],
            col_serial: 0,
            play_position: 0,
            total_samples: 0,
        }
    }
}

// ── Rodio source ─────────────────────────────────────────────────────────────

pub struct ChannelSource {
    pub rx: mpsc::Receiver<f32>,
    pub sample_rate: u32,
}

impl Iterator for ChannelSource {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        self.rx.recv().ok()
    }
}

impl rodio::Source for ChannelSource {
    fn current_frame_len(&self) -> Option<usize> { None }
    fn channels(&self) -> u16 { 1 }
    fn sample_rate(&self) -> u32 { self.sample_rate }
    fn total_duration(&self) -> Option<std::time::Duration> { None }
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Spawn a DSP thread reading from a pre-loaded file buffer.
pub fn start_dsp_thread(
    raw_samples: Vec<f32>,
    sample_rate: u32,
    start_sample: usize,
    params: Arc<Mutex<DspParams>>,
    viz: Arc<Mutex<VisualizationSnapshot>>,
    stop: Arc<AtomicBool>,
    midi: Option<Arc<MidiTimeline>>,
) -> (std::thread::JoinHandle<()>, ChannelSource) {
    let buf = (sample_rate as usize * 15 / 100).next_power_of_two();
    let (tx, rx) = mpsc::sync_channel::<f32>(buf);
    let source = ChannelSource { rx, sample_rate };
    let handle = std::thread::spawn(move || {
        file_dsp_loop(raw_samples, sample_rate, start_sample, params, viz, stop, midi, tx);
    });
    (handle, source)
}

/// Spawn a DSP thread reading live from a microphone ring buffer.
pub fn start_mic_dsp_thread(
    mic_ring: MicRing,
    sample_rate: u32,
    params: Arc<Mutex<DspParams>>,
    viz: Arc<Mutex<VisualizationSnapshot>>,
    stop: Arc<AtomicBool>,
    midi: Option<Arc<MidiTimeline>>,
) -> (std::thread::JoinHandle<()>, ChannelSource) {
    let buf = (sample_rate as usize * 15 / 100).next_power_of_two();
    let (tx, rx) = mpsc::sync_channel::<f32>(buf);
    let source = ChannelSource { rx, sample_rate };
    let handle = std::thread::spawn(move || {
        mic_dsp_loop(mic_ring, sample_rate, params, viz, stop, midi, tx);
    });
    (handle, source)
}

// ── Shared DSP frame processor ────────────────────────────────────────────────
//
// Called once per STFT hop with the raw input block already in `block`.
// Mutates `block` (bitcrusher may touch it), fills `out_mags`, and writes
// to viz. Returns the processed output slice (borrowed from internal state).

struct FrameState {
    phases_in: [f32; N],
    phases_out: [f32; N],
    harm_env: [f32; HALF],
    harm_inv: [f32; HALF],
    carrier_osc: Oscillator,
    acc_in: [f32; HALF],
    frame_count: u32,
    hann: [f32; N],
}

impl FrameState {
    fn new(carrier_freq: f32, sample_rate: f32, carrier_waveform: synthphone_e_vocal_dsp::audio::Waveform) -> Self {
        Self {
            phases_in: [0.0; N],
            phases_out: [0.0; N],
            harm_env: [0.0; HALF],
            harm_inv: [0.0; HALF],
            carrier_osc: Oscillator::new(carrier_freq, sample_rate, carrier_waveform),
            acc_in: [0.0; HALF],
            frame_count: 0,
            hann: *Fft1024::get_hann_window(),
        }
    }

    /// Process one STFT block. Returns output frame and (input_mags, output_mags, formant_env).
    fn process_frame(
        &mut self,
        raw_block: &[f32; N],
        cur: &DspParams,
        play_position: usize,
        midi: &Option<Arc<MidiTimeline>>,
        bypassed: bool,
    ) -> ([f32; N], [f32; HALF], [f32; HALF], [f32; HALF]) {
        self.carrier_osc.set_freq(cur.carrier_freq);
        self.carrier_osc.set_waveform(cur.carrier_waveform);

        let mut block = *raw_block;

        // Input FFT for viz
        let mut win_in = block;
        for (s, &w) in win_in.iter_mut().zip(self.hann.iter()) { *s *= w; }
        let fft_in = Fft1024::forward_fft(&mut win_in);
        let mut fft_in_mags = [0.0f32; HALF];
        for (i, c) in fft_in.iter().take(HALF).enumerate() {
            fft_in_mags[i] = (c.re * c.re + c.im * c.im).sqrt();
            self.acc_in[i] += fft_in_mags[i];
        }
        self.frame_count += 1;

        // If bypassing, pass raw through
        if bypassed {
            let fft_out_mags = fft_in_mags;
            return (block, fft_in_mags, fft_out_mags, [0.0; HALF]);
        }

        // Bitcrusher pre-processing
        if cur.bit_depth < 16 || cur.sample_rate_div > 1 {
            let mut hold = 0i32;
            let mut held = 0.0f32;
            for s in block.iter_mut() {
                let crushed = if cur.bit_depth < 16 { bitcrush(*s, cur.bit_depth) } else { *s };
                *s = if cur.sample_rate_div > 1 {
                    sample_rate_reduce(crushed, cur.sample_rate_div, &mut hold, &mut held)
                } else { crushed };
            }
        }

        // Harmony: inject MIDI target frequency
        let mut settings = cur.settings;
        if settings.mode == ProcessingMode::Harmony {
            if let Some(timeline) = midi {
                let freq = timeline.freq_at(play_position);
                settings.midi_frequencies = [0.0; 8];
                if freq > 0.0 {
                    settings.midi_frequencies[0] = freq;
                }
            }
        }

        // DSP
        let result = if settings.mode == ProcessingMode::Vocode {
            let mut carrier = [0.0f32; N];
            for s in carrier.iter_mut() { *s = self.carrier_osc.next_value(); }
            process_vocal_effects_1024(
                &mut block, Some(&mut carrier),
                &mut self.phases_in, &mut self.phases_out,
                &mut self.harm_env, &mut self.harm_inv,
                1.0, &cur.config, &settings,
            )
        } else {
            process_vocal_effects_1024(
                &mut block, None,
                &mut self.phases_in, &mut self.phases_out,
                &mut self.harm_env, &mut self.harm_inv,
                1.0, &cur.config, &settings,
            )
        };

        // Output FFT for viz
        let mut win_out = result;
        for (s, &w) in win_out.iter_mut().zip(self.hann.iter()) { *s *= w; }
        let fft_out = Fft1024::forward_fft(&mut win_out);
        let mut fft_out_mags = [0.0f32; HALF];
        for (i, c) in fft_out.iter().take(HALF).enumerate() {
            fft_out_mags[i] = (c.re * c.re + c.im * c.im).sqrt();
        }

        // Cepstral formant envelope
        let scale = 1.0 / self.frame_count as f32;
        let mut avg_in = [0.0f32; HALF];
        for (a, &b) in avg_in.iter_mut().zip(self.acc_in.iter()) { *a = b * scale; }
        let mut formant_env = [0.0f32; HALF];
        extract_cepstral_envelope::<N, HALF, Fft1024>(&avg_in, &mut formant_env);

        (result, fft_in_mags, fft_out_mags, formant_env)
    }
}

fn write_viz(
    viz: &Arc<Mutex<VisualizationSnapshot>>,
    raw_block: &[f32; N],
    result: &[f32; N],
    fft_in: &[f32; HALF],
    fft_out: &[f32; HALF],
    formant: &[f32; HALF],
    pitch_hz: f32,
    output_pitch_hz: f32,
    play_pos: usize,
) {
    let mut v = viz.lock().unwrap();
    v.input_window.copy_from_slice(raw_block);
    v.output_window.copy_from_slice(result);
    v.input_fft.copy_from_slice(fft_in);
    v.output_fft.copy_from_slice(fft_out);
    v.formant_envelope.copy_from_slice(formant);
    v.pitch_hz = pitch_hz;
    v.output_pitch_hz = output_pitch_hz;
    v.input_spec_col.copy_from_slice(fft_in);
    v.output_spec_col.copy_from_slice(fft_out);
    v.col_serial += 1;
    v.play_position = play_pos;
}

// ── File DSP loop ─────────────────────────────────────────────────────────────

fn file_dsp_loop(
    raw_samples: Vec<f32>,
    sample_rate: u32,
    start_sample: usize,
    params: Arc<Mutex<DspParams>>,
    viz: Arc<Mutex<VisualizationSnapshot>>,
    stop: Arc<AtomicBool>,
    midi: Option<Arc<MidiTimeline>>,
    audio_tx: mpsc::SyncSender<f32>,
) {
    let total = raw_samples.len();
    let hop = params.lock().unwrap().config.hop_size.max(1) as usize;

    let pre_pad = N - hop;
    let mut padded = vec![0.0f32; pre_pad + total + N];
    padded[pre_pad..pre_pad + total].copy_from_slice(&raw_samples);

    let mut ola_out = vec![0.0f32; padded.len() + N];

    let (init_freq, init_wave) = {
        let p = params.lock().unwrap();
        (p.carrier_freq, p.carrier_waveform)
    };
    let mut state = FrameState::new(init_freq, sample_rate as f32, init_wave);

    let start_hop = (start_sample / hop) * hop;
    let mut pos = pre_pad + start_hop;

    viz.lock().unwrap().total_samples = total;

    'frame: while pos + N <= padded.len() {
        if stop.load(Ordering::Relaxed) { break; }

        let cur = params.lock().unwrap().clone();
        let hop_now = cur.config.hop_size.max(1) as usize;

        let bypassed = cur.bypass_until.map(|t| std::time::Instant::now() < t).unwrap_or(false);

        let mut raw_block = [0.0f32; N];
        raw_block.copy_from_slice(&padded[pos..pos + N]);

        let pitch_hz = find_pitch_yin(&raw_block, &cur.config);
        let play_pos = (pos - pre_pad).min(total);

        let (result, fft_in, fft_out, formant) =
            state.process_frame(&raw_block, &cur, play_pos, &midi, bypassed);

        let output_pitch_hz = find_pitch_yin(&result, &cur.config);

        // OLA
        for (i, &s) in result.iter().enumerate() {
            let idx = pos + i;
            if idx < ola_out.len() { ola_out[idx] += s; }
        }

        // Emit audio
        if pos >= pre_pad {
            for i in 0..hop_now {
                let idx = pos + i;
                if idx < ola_out.len() {
                    if audio_tx.send(ola_out[idx]).is_err() { break 'frame; }
                }
            }
        }

        write_viz(&viz, &raw_block, &result, &fft_in, &fft_out, &formant,
                  pitch_hz, output_pitch_hz, play_pos);

        pos += hop_now;
    }

    if !stop.load(Ordering::Relaxed) {
        if let Ok(mut v) = viz.lock() { v.play_position = total; }
    }
}

// ── Mic DSP loop ──────────────────────────────────────────────────────────────

fn mic_dsp_loop(
    mic_ring: MicRing,
    sample_rate: u32,
    params: Arc<Mutex<DspParams>>,
    viz: Arc<Mutex<VisualizationSnapshot>>,
    stop: Arc<AtomicBool>,
    midi: Option<Arc<MidiTimeline>>,
    audio_tx: mpsc::SyncSender<f32>,
) {
    let (init_freq, init_wave, hop) = {
        let p = params.lock().unwrap();
        (p.carrier_freq, p.carrier_waveform, p.config.hop_size.max(1) as usize)
    };
    let mut state = FrameState::new(init_freq, sample_rate as f32, init_wave);

    // Rolling OLA output buffer: length 2*N is enough for overlap-add.
    // Every hop we shift left by `hop` and zero the tail.
    let mut ola_buf = vec![0.0f32; N + hop];

    // Input ring: we maintain a rolling N-sample window in `window`.
    // Each iteration we shift it left by `hop` and fill the tail from mic_ring.
    let mut window = vec![0.0f32; N];

    // Monotonic sample counter (for MIDI timeline).
    let mut play_pos: usize = 0;

    viz.lock().unwrap().total_samples = 0; // 0 = no file / live

    loop {
        if stop.load(Ordering::Relaxed) { break; }

        let cur = params.lock().unwrap().clone();
        let hop_now = cur.config.hop_size.max(1) as usize;

        // Block until hop_now new samples arrive from the mic.
        loop {
            if stop.load(Ordering::Relaxed) { return; }
            if mic_ring.lock().unwrap().len() >= hop_now { break; }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        // Shift window left, fill tail with new mic samples.
        window.copy_within(hop_now.., 0);
        {
            let mut ring = mic_ring.lock().unwrap();
            for s in window[N - hop_now..].iter_mut() {
                *s = ring.pop_front().unwrap_or(0.0);
            }
        }

        let bypassed = cur.bypass_until.map(|t| std::time::Instant::now() < t).unwrap_or(false);

        let mut raw_block = [0.0f32; N];
        raw_block.copy_from_slice(&window);

        let pitch_hz = find_pitch_yin(&raw_block, &cur.config);

        let (result, fft_in, fft_out, formant) =
            state.process_frame(&raw_block, &cur, play_pos, &midi, bypassed);

        let output_pitch_hz = find_pitch_yin(&result, &cur.config);

        // OLA: shift buffer left by hop_now, add new frame.
        ola_buf.copy_within(hop_now.., 0);
        for s in ola_buf[N..].iter_mut() { *s = 0.0; }
        for (i, &s) in result.iter().enumerate() {
            if i < ola_buf.len() { ola_buf[i] += s; }
        }

        // Emit hop_now samples.
        for &s in &ola_buf[..hop_now] {
            if audio_tx.send(s).is_err() { return; }
        }

        write_viz(&viz, &raw_block, &result, &fft_in, &fft_out, &formant,
                  pitch_hz, output_pitch_hz, play_pos);

        play_pos += hop_now;
    }
}
