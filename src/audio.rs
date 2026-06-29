use synthphone_e_vocal_dsp::{
    MusicalSettings, VocalEffectsConfig, process_vocal_effects_1024,
    audio::{Oscillator, Waveform},
    dsp::{bitcrush, extract_cepstral_envelope, find_pitch_yin, sample_rate_reduce, Fft1024, FftOps},
};

// ── Public DSP parameter bundle ───────────────────────────────────────────────

pub struct DspParams {
    pub config: VocalEffectsConfig,
    pub settings: MusicalSettings,
    /// Bitcrusher bit depth (1–16). 16 = pass-through (no crush).
    pub bit_depth: i8,
    /// Sample-rate reduction factor (1–32). 1 = no reduction.
    pub sample_rate_div: i8,
    /// Vocoder carrier frequency in Hz.
    pub carrier_freq: f32,
    /// Vocoder carrier waveform.
    pub carrier_waveform: Waveform,
}

// ── Audio data ────────────────────────────────────────────────────────────────

pub struct AudioData {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

#[derive(Debug, Clone)]
pub struct ProcessedAudio {
    pub samples: Vec<f32>,
    pub avg_input_spectrum: Vec<f32>,
    pub avg_output_spectrum: Vec<f32>,
    pub formant_envelope: Vec<f32>,
    pub spectrogram_frames: Vec<Vec<f32>>,
    pub pitch_track_hz: Vec<f32>,
}

// ── File loading ──────────────────────────────────────────────────────────────

pub fn load_wav(
    path: &std::path::Path,
) -> Result<AudioData, Box<dyn std::error::Error + Send + Sync>> {
    let reader = hound::WavReader::open(path)?;
    let spec = reader.spec();

    let raw: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .into_samples::<f32>()
            .collect::<Result<Vec<_>, _>>()?,
        hound::SampleFormat::Int => {
            let scale = (1i64 << (spec.bits_per_sample - 1)) as f32;
            if spec.bits_per_sample <= 16 {
                reader
                    .into_samples::<i16>()
                    .collect::<Result<Vec<_>, _>>()?
                    .into_iter()
                    .map(|s| s as f32 / scale)
                    .collect()
            } else {
                reader
                    .into_samples::<i32>()
                    .collect::<Result<Vec<_>, _>>()?
                    .into_iter()
                    .map(|s| s as f32 / scale)
                    .collect()
            }
        }
    };

    let ch = spec.channels as usize;
    let samples: Vec<f32> = if ch > 1 {
        raw.chunks_exact(ch)
            .map(|frame| frame.iter().sum::<f32>() / ch as f32)
            .collect()
    } else {
        raw
    };

    Ok(AudioData { samples, sample_rate: spec.sample_rate })
}

// ── Processing ────────────────────────────────────────────────────────────────

pub fn process_samples(samples: &[f32], params: &DspParams) -> ProcessedAudio {
    const N: usize = 1024;
    const HALF: usize = N / 2;
    let hop = params.config.hop_size.max(1);

    // ── Bitcrusher / sample-rate-reduction pre-processing ─────────────────────
    // Applied sample-by-sample before any STFT processing.
    let input: Vec<f32> = if params.bit_depth < 16 || params.sample_rate_div > 1 {
        let mut hold_counter = 0i32;
        let mut held_value = 0.0f32;
        let mut out = samples.to_vec();
        for s in out.iter_mut() {
            let crushed =
                if params.bit_depth < 16 { bitcrush(*s, params.bit_depth) } else { *s };
            *s = if params.sample_rate_div > 1 {
                sample_rate_reduce(
                    crushed,
                    params.sample_rate_div,
                    &mut hold_counter,
                    &mut held_value,
                )
            } else {
                crushed
            };
        }
        out
    } else {
        samples.to_vec()
    };

    // ── STFT overlap-add setup ────────────────────────────────────────────────
    let mut phases_in = [0.0f32; N];
    let mut phases_out = [0.0f32; N];
    let mut harm_env = [0.0f32; HALF];
    let mut harm_inv = [0.0f32; HALF];

    let pre_pad = N - hop;
    let padded_len = pre_pad + input.len() + N;
    let mut padded = vec![0.0f32; padded_len];
    padded[pre_pad..pre_pad + input.len()].copy_from_slice(&input);

    let mut output = vec![0.0f32; padded_len];

    let hann = Fft1024::get_hann_window();
    let mut acc_in = [0.0f32; HALF];
    let mut acc_out = [0.0f32; HALF];
    let mut frame_count = 0u32;

    const MAX_SPEC_FRAMES: usize = 4096;
    let expected_frames = padded_len / hop + 1;
    let spec_stride = (expected_frames / MAX_SPEC_FRAMES).max(1);

    let mut spectrogram_frames: Vec<Vec<f32>> = Vec::new();
    let mut pitch_track_hz: Vec<f32> = Vec::new();

    // Stateful oscillator for vocoder carrier, initialised once before the loop.
    let is_vocode = params.settings.mode
        == synthphone_e_vocal_dsp::ProcessingMode::Vocode;
    let mut carrier_osc = is_vocode.then(|| {
        Oscillator::new(
            params.carrier_freq,
            params.config.sample_rate,
            params.carrier_waveform,
        )
    });

    let mut pos = 0;
    while pos + N <= padded_len {
        let mut block = [0.0f32; N];
        block.copy_from_slice(&padded[pos..pos + N]);

        // YIN pitch detection on the raw (pre-windowed) block.
        let pitch_hz = find_pitch_yin(&block, &params.config);

        // Input spectrum.
        let mut win_in = block;
        for (s, &w) in win_in.iter_mut().zip(hann.iter()) {
            *s *= w;
        }
        let fft_in = Fft1024::forward_fft(&mut win_in);
        let mut frame_spec = [0.0f32; HALF];
        for (i, c) in fft_in.iter().take(HALF).enumerate() {
            let mag = (c.re * c.re + c.im * c.im).sqrt();
            acc_in[i] += mag;
            frame_spec[i] = mag;
        }

        if frame_count as usize % spec_stride == 0 {
            spectrogram_frames.push(frame_spec.to_vec());
            pitch_track_hz.push(pitch_hz);
        }

        // DSP processing.
        let result = if let Some(ref mut osc) = carrier_osc {
            let mut carrier = [0.0f32; N];
            for s in carrier.iter_mut() {
                *s = osc.next_value();
            }
            process_vocal_effects_1024(
                &mut block,
                Some(&mut carrier),
                &mut phases_in,
                &mut phases_out,
                &mut harm_env,
                &mut harm_inv,
                1.0,
                &params.config,
                &params.settings,
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
                &params.config,
                &params.settings,
            )
        };

        for (i, &s) in result.iter().enumerate() {
            if pos + i < output.len() {
                output[pos + i] += s;
            }
        }

        // Output spectrum.
        let mut win_out = result;
        for (s, &w) in win_out.iter_mut().zip(hann.iter()) {
            *s *= w;
        }
        let fft_out = Fft1024::forward_fft(&mut win_out);
        for (i, c) in fft_out.iter().take(HALF).enumerate() {
            acc_out[i] += (c.re * c.re + c.im * c.im).sqrt();
        }

        frame_count += 1;
        pos += hop;
    }

    let scale = if frame_count > 0 { 1.0 / frame_count as f32 } else { 1.0 };
    let avg_input_spectrum: Vec<f32> = acc_in.iter().map(|&s| s * scale).collect();
    let avg_output_spectrum: Vec<f32> = acc_out.iter().map(|&s| s * scale).collect();

    let mut avg_mag_arr = [0.0f32; HALF];
    for (slot, &v) in avg_mag_arr.iter_mut().zip(avg_input_spectrum.iter()) {
        *slot = v;
    }
    let mut formant_env_arr = [0.0f32; HALF];
    extract_cepstral_envelope::<N, HALF, Fft1024>(&avg_mag_arr, &mut formant_env_arr);

    let end = (pre_pad + input.len()).min(output.len());
    ProcessedAudio {
        samples: output[pre_pad..end].to_vec(),
        avg_input_spectrum,
        avg_output_spectrum,
        formant_envelope: formant_env_arr.to_vec(),
        spectrogram_frames,
        pitch_track_hz,
    }
}
