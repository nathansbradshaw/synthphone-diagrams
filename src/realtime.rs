/// Real-time DSP engine.
///
/// Mirrors the SynthPhone's interrupt-driven pipeline:
///   audio ISR pushes samples into in_ring → every HOP_SIZE samples the FFT task
///   reads a 1024-sample window, runs process_vocal_effects_1024, and OLA-adds the
///   result into out_ring.
///
/// Here, instead of hardware rings, we:
///   • read from a pre-loaded Vec<f32>
///   • push processed samples into a sync_channel (rodio drains the other end)
///   • write per-frame viz data into Arc<Mutex<VisualizationSnapshot>>
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc,
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

pub const N: usize = 1024;
pub const HALF: usize = N / 2;

// ── Shared visualization data (DSP thread writes, GUI polls) ─────────────────

pub struct VisualizationSnapshot {
    pub input_window: Vec<f32>,     // N raw input samples (current block)
    pub output_window: Vec<f32>,    // N synthesis output samples
    pub input_fft: Vec<f32>,        // HALF magnitude bins
    pub output_fft: Vec<f32>,       // HALF magnitude bins
    pub formant_envelope: Vec<f32>, // HALF cepstral envelope bins
    pub pitch_hz: f32,
    pub output_pitch_hz: f32,
    pub spectrogram_col: Vec<f32>, // HALF – latest input FFT column
    pub col_serial: u64,           // increments every frame; GUI compares to detect new cols
    pub play_position: usize,      // sample index in the original (unpadded) audio
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
            spectrogram_col: vec![0.0; HALF],
            col_serial: 0,
            play_position: 0,
            total_samples: 0,
        }
    }
}

// ── Rodio source backed by the DSP channel ────────────────────────────────────

pub struct ChannelSource {
    pub rx: mpsc::Receiver<f32>,
    pub sample_rate: u32,
}

impl Iterator for ChannelSource {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        // Blocking recv paces rodio to real-time. Returns None once the DSP
        // thread drops tx (stop or end-of-file), which tells rodio the
        // source is finished.
        self.rx.recv().ok()
    }
}

impl rodio::Source for ChannelSource {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> u16 {
        1
    }
    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
    fn total_duration(&self) -> Option<std::time::Duration> {
        None
    }
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Spawns the DSP thread. Returns its handle and the audio channel source that
/// should be handed to rodio.
pub fn start_dsp_thread(
    raw_samples: Vec<f32>,
    sample_rate: u32,
    start_sample: usize,
    params: Arc<Mutex<DspParams>>,
    viz: Arc<Mutex<VisualizationSnapshot>>,
    stop: Arc<AtomicBool>,
) -> (std::thread::JoinHandle<()>, ChannelSource) {
    // ~150 ms buffer. When full the DSP thread blocks, naturally pacing it to
    // real-time speed while keeping visualization within one GUI tick of audio.
    let buf_samples = (sample_rate as usize * 15 / 100).next_power_of_two();
    let (tx, rx) = mpsc::sync_channel::<f32>(buf_samples);

    let source = ChannelSource { rx, sample_rate };
    let handle = std::thread::spawn(move || {
        dsp_loop(
            raw_samples,
            sample_rate,
            start_sample,
            params,
            viz,
            stop,
            tx,
        );
    });
    (handle, source)
}

// ── DSP loop (runs on its own thread) ────────────────────────────────────────

fn dsp_loop(
    raw_samples: Vec<f32>,
    sample_rate: u32,
    start_sample: usize,
    params: Arc<Mutex<DspParams>>,
    viz: Arc<Mutex<VisualizationSnapshot>>,
    stop: Arc<AtomicBool>,
    audio_tx: mpsc::SyncSender<f32>,
) {
    let total = raw_samples.len();
    let hop = params.lock().unwrap().config.hop_size.max(1) as usize;

    // Pre-pad with N-hop zeros so the first real sample gets a full window.
    let pre_pad = N - hop;
    let mut padded = vec![0.0f32; pre_pad + total + N];
    padded[pre_pad..pre_pad + total].copy_from_slice(&raw_samples);

    // OLA accumulator (same size as padded + one extra window).
    let mut ola_out = vec![0.0f32; padded.len() + N];

    // STFT phase state — persistent across frames exactly as on the SynthPhone.
    let mut phases_in = [0.0f32; N];
    let mut phases_out = [0.0f32; N];
    let mut harm_env = [0.0f32; HALF];
    let mut harm_inv = [0.0f32; HALF];

    // Carrier oscillator for vocoder — phase is continuous across frames.
    let (init_freq, init_wave) = {
        let p = params.lock().unwrap();
        (p.carrier_freq, p.carrier_waveform)
    };
    let mut carrier_osc = Oscillator::new(init_freq, sample_rate as f32, init_wave);

    // Running accumulator for the cepstral formant envelope.
    let mut acc_in = [0.0f32; HALF];
    let mut frame_count = 0u32;

    let hann = Fft1024::get_hann_window();

    // Align start position to a hop boundary inside the padded array.
    let start_hop = (start_sample / hop) * hop;
    let mut pos = pre_pad + start_hop;

    viz.lock().unwrap().total_samples = total;

    'frame: while pos + N <= padded.len() {
        if stop.load(Ordering::Relaxed) {
            break;
        }

        // Snapshot current params (cheap clone, ~200 bytes).
        let cur = params.lock().unwrap().clone();
        let hop_now = cur.config.hop_size.max(1) as usize;

        // Update carrier oscillator live (takes effect next frame).
        carrier_osc.set_freq(cur.carrier_freq);
        carrier_osc.set_waveform(cur.carrier_waveform);

        // ── Read input block ──────────────────────────────────────────────────
        let mut block = [0.0f32; N];
        block.copy_from_slice(&padded[pos..pos + N]);

        // Keep a copy before DSP may modify block.
        let raw_block = block;

        // ── Pitch detection (must run on the pre-windowed raw block) ──────────
        let pitch_hz = find_pitch_yin(&block, &cur.config);

        // ── Input FFT for visualization ───────────────────────────────────────
        let mut win_in = block;
        for (s, &w) in win_in.iter_mut().zip(hann.iter()) {
            *s *= w;
        }
        let fft_in = Fft1024::forward_fft(&mut win_in);
        let mut fft_in_mags = [0.0f32; HALF];
        for (i, c) in fft_in.iter().take(HALF).enumerate() {
            fft_in_mags[i] = (c.re * c.re + c.im * c.im).sqrt();
            acc_in[i] += fft_in_mags[i];
        }
        frame_count += 1;

        // ── Bitcrusher pre-processing (sample-level, before STFT) ────────────
        if cur.bit_depth < 16 || cur.sample_rate_div > 1 {
            let mut hold = 0i32;
            let mut held = 0.0f32;
            for s in block.iter_mut() {
                let crushed = if cur.bit_depth < 16 {
                    bitcrush(*s, cur.bit_depth)
                } else {
                    *s
                };
                *s = if cur.sample_rate_div > 1 {
                    sample_rate_reduce(crushed, cur.sample_rate_div, &mut hold, &mut held)
                } else {
                    crushed
                };
            }
        }

        // ── DSP (phase vocoder / vocoder / formant shift) ─────────────────────
        let result = if cur.settings.mode == ProcessingMode::Vocode {
            let mut carrier = [0.0f32; N];
            for s in carrier.iter_mut() {
                *s = carrier_osc.next_value();
            }
            process_vocal_effects_1024(
                &mut block,
                Some(&mut carrier),
                &mut phases_in,
                &mut phases_out,
                &mut harm_env,
                &mut harm_inv,
                1.0,
                &cur.config,
                &cur.settings,
            )
        } else {
            process_vocal_effects_1024(
                &mut block,
                None,
                &mut phases_in,
                &mut phases_out,
                &mut harm_env,
                &mut harm_inv,
                1.0,
                &cur.config,
                &cur.settings,
            )
        };

        // ── Output pitch detection ────────────────────────────────────────────
        let output_pitch_hz = find_pitch_yin(&result, &cur.config);

        // ── Overlap-add into accumulator ──────────────────────────────────────
        for (i, &s) in result.iter().enumerate() {
            let idx = pos + i;
            if idx < ola_out.len() {
                ola_out[idx] += s;
            }
        }

        // ── Output FFT for visualization ──────────────────────────────────────
        let mut win_out = result;
        for (s, &w) in win_out.iter_mut().zip(hann.iter()) {
            *s *= w;
        }
        let fft_out = Fft1024::forward_fft(&mut win_out);
        let mut fft_out_mags = [0.0f32; HALF];
        for (i, c) in fft_out.iter().take(HALF).enumerate() {
            fft_out_mags[i] = (c.re * c.re + c.im * c.im).sqrt();
        }

        // ── Cepstral formant envelope (running average over all frames) ───────
        let scale = 1.0 / frame_count as f32;
        let mut avg_in = [0.0f32; HALF];
        for (a, &b) in avg_in.iter_mut().zip(acc_in.iter()) {
            *a = b * scale;
        }
        let mut formant_env = [0.0f32; HALF];
        extract_cepstral_envelope::<N, HALF, Fft1024>(&avg_in, &mut formant_env);

        // ── Emit audio samples ────────────────────────────────────────────────
        // After the OLA add above, ola_out[pos..pos+hop] is fully accumulated:
        // every frame whose synthesis window overlaps those positions has now run.
        // The first pre_pad samples are pre-roll zeros we skip.
        if pos >= pre_pad {
            for i in 0..hop_now {
                let idx = pos + i;
                if idx < ola_out.len() {
                    if audio_tx.send(ola_out[idx]).is_err() {
                        break 'frame; // rodio dropped its end → stop
                    }
                }
            }
        }

        // ── Update visualization snapshot ─────────────────────────────────────
        let play_pos = (pos - pre_pad).min(total);
        {
            let mut v = viz.lock().unwrap();
            v.input_window.copy_from_slice(&raw_block);
            v.output_window.copy_from_slice(&result);
            v.input_fft.copy_from_slice(&fft_in_mags);
            v.output_fft.copy_from_slice(&fft_out_mags);
            v.formant_envelope.copy_from_slice(&formant_env);
            v.pitch_hz = pitch_hz;
            v.output_pitch_hz = output_pitch_hz;
            v.spectrogram_col.copy_from_slice(&fft_in_mags);
            v.col_serial += 1;
            v.play_position = play_pos;
        }

        pos += hop_now;
    }

    // Only signal end-of-file if the loop finished naturally (not via stop flag).
    if !stop.load(Ordering::Relaxed) {
        if let Ok(mut v) = viz.lock() {
            v.play_position = total;
        }
    }
}
