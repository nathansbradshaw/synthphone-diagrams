use synthphone_e_vocal_dsp::{MusicalSettings, VocalEffectsConfig, audio::Waveform};

// ── Public DSP parameter bundle ───────────────────────────────────────────────

#[derive(Clone)]
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
    /// When set, DSP effects are bypassed until this instant.
    pub bypass_until: Option<std::time::Instant>,
    /// Vocoder mode only: drive the carrier frequency from the loaded MIDI
    /// timeline instead of the static `carrier_freq`.
    pub midi_driven_carrier: bool,
}

// ── Audio data ────────────────────────────────────────────────────────────────

pub struct AudioData {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
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

    Ok(AudioData {
        samples,
        sample_rate: spec.sample_rate,
    })
}
