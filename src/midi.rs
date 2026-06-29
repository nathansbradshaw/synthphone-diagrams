/// MIDI file timeline: maps playback sample-position → active note frequency.
use std::path::Path;

pub struct MidiTimeline {
    /// Sorted (sample_offset, frequency_hz) pairs. Frequency 0.0 = rest/note-off.
    events: Vec<(usize, f32)>,
}

impl MidiTimeline {
    pub fn load(path: &Path, sample_rate: u32) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let data = std::fs::read(path)?;
        let smf = midly::Smf::parse(&data)?;

        let ticks_per_beat = match smf.header.timing {
            midly::Timing::Metrical(t) => t.as_int() as u64,
            midly::Timing::Timecode(fps, sub) => (fps.as_f32() * sub as f32) as u64,
        };

        // Collect all events from all tracks, merged with absolute tick times.
        let mut all_events: Vec<(u64, midly::TrackEventKind<'_>)> = Vec::new();
        for track in &smf.tracks {
            let mut abs_tick: u64 = 0;
            for ev in track {
                abs_tick += ev.delta.as_int() as u64;
                all_events.push((abs_tick, ev.kind));
            }
        }
        all_events.sort_by_key(|(t, _)| *t);

        // Walk events, tracking tempo and emitting (sample_offset, hz) pairs.
        let mut us_per_beat: u64 = 500_000; // default 120 BPM
        let mut result: Vec<(usize, f32)> = Vec::new();
        // Track which notes are currently on per channel+key.
        let mut active: std::collections::HashMap<(u8, u8), f32> = Default::default();

        let tick_to_sample = |tick: u64, uspb: u64| -> usize {
            ((tick * uspb / ticks_per_beat) as f64 / 1_000_000.0 * sample_rate as f64) as usize
        };

        for (tick, kind) in &all_events {
            match kind {
                midly::TrackEventKind::Meta(midly::MetaMessage::Tempo(t)) => {
                    us_per_beat = t.as_int() as u64;
                }
                midly::TrackEventKind::Midi { channel, message } => {
                    let sample = tick_to_sample(*tick, us_per_beat);
                    match message {
                        midly::MidiMessage::NoteOn { key, vel } if vel.as_int() > 0 => {
                            let hz = midi_note_to_hz(key.as_int());
                            active.insert((channel.as_int(), key.as_int()), hz);
                            // Emit the highest active note as the target.
                            let top = active.values().copied().fold(0.0_f32, f32::max);
                            result.push((sample, top));
                        }
                        midly::MidiMessage::NoteOn { key, .. }
                        | midly::MidiMessage::NoteOff { key, .. } => {
                            active.remove(&(channel.as_int(), key.as_int()));
                            let top = active.values().copied().fold(0.0_f32, f32::max);
                            result.push((sample, top));
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }

        result.sort_by_key(|(s, _)| *s);
        Ok(Self { events: result })
    }

    /// Frequency (Hz) of the active note at `sample_pos`. Returns 0.0 during rests.
    pub fn freq_at(&self, sample_pos: usize) -> f32 {
        if self.events.is_empty() {
            return 0.0;
        }
        let idx = self.events.partition_point(|(s, _)| *s <= sample_pos);
        if idx == 0 { 0.0 } else { self.events[idx - 1].1 }
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

fn midi_note_to_hz(note: u8) -> f32 {
    440.0 * 2.0_f32.powf((note as f32 - 69.0) / 12.0)
}
