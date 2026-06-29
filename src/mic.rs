/// Live microphone capture via cpal.
///
/// Fills a shared VecDeque ring that the DSP thread drains.
/// Paces the DSP loop to real-time by withholding samples until they arrive.
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

pub type MicRing = Arc<Mutex<VecDeque<f32>>>;

pub struct MicCapture {
    // Keep the stream alive — dropping it stops capture.
    _stream: cpal::Stream,
    pub ring: MicRing,
    pub sample_rate: u32,
}

impl MicCapture {
    pub fn open() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or("no default input device")?;

        // Find a supported config closest to 48 kHz mono f32.
        let supported = device.default_input_config()?;
        let sample_rate = supported.sample_rate().0;
        let channels = supported.channels() as usize;

        let ring: MicRing = Arc::new(Mutex::new(VecDeque::with_capacity(8192)));
        let ring_cb = Arc::clone(&ring);

        let stream = device.build_input_stream(
            &supported.into(),
            move |data: &[f32], _| {
                let mut r = ring_cb.lock().unwrap();
                // Down-mix to mono on the fly.
                if channels == 1 {
                    r.extend(data.iter().copied());
                } else {
                    for frame in data.chunks(channels) {
                        let mono = frame.iter().sum::<f32>() / channels as f32;
                        r.push_back(mono);
                    }
                }
                // Cap ring at ~2 s to avoid unbounded growth during pauses.
                let cap = (sample_rate as usize) * 2;
                while r.len() > cap {
                    r.pop_front();
                }
            },
            |err| eprintln!("mic stream error: {err}"),
            None,
        )?;

        stream.play()?;
        Ok(Self { _stream: stream, ring, sample_rate })
    }
}
