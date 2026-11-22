use midly::{MetaMessage, MidiMessage, Smf, Timing, TrackEventKind};
/// Precomputed chord changes: one binary search per hop, no audio-thread allocation.
pub struct Timeline {
    events: Vec<(usize, [f32; 8])>,
    length: usize,
}
impl Timeline {
    pub fn parse(bytes: &[u8], sample_rate: f32) -> Result<Self, String> {
        let smf = Smf::parse(bytes).map_err(|e| e.to_string())?;
        if smf.header.format == midly::Format::Sequential {
            return Err("MIDI format 2 is not a shared timeline. Export format 0 or 1.".into());
        }
        let mut all = Vec::new();
        for track in &smf.tracks {
            let mut tick = 0u64;
            for event in track {
                tick += event.delta.as_int() as u64;
                all.push((tick, event.kind));
            }
        }
        all.sort_by_key(|e| e.0);
        let mut events = vec![(0, [0.; 8])];
        let (mut previous, mut seconds, mut tempo) = (0u64, 0f64, 500000.);
        let mut active = [[false; 128]; 16];
        for (tick, kind) in all {
            seconds += match smf.header.timing {
                Timing::Metrical(t) => (tick - previous) as f64 * tempo / (t.as_int() as f64 * 1e6),
                Timing::Timecode(fps, sub) => {
                    (tick - previous) as f64 / (fps.as_f32() as f64 * sub as f64)
                }
            };
            previous = tick;
            if let TrackEventKind::Meta(MetaMessage::Tempo(t)) = kind {
                tempo = t.as_int() as f64;
            }
            if let TrackEventKind::Midi { channel, message } = kind {
                let channel = channel.as_int() as usize;
                if channel == 9 {
                    continue;
                }
                match message {
                    MidiMessage::NoteOn { key, vel } => {
                        active[channel][key.as_int() as usize] = vel.as_int() > 0
                    }
                    MidiMessage::NoteOff { key, .. } => {
                        active[channel][key.as_int() as usize] = false
                    }
                    MidiMessage::Controller { controller, .. }
                        if controller.as_int() == 120 || controller.as_int() == 123 =>
                    {
                        active[channel].fill(false)
                    }
                    _ => continue,
                }
                let mut chord = [0.; 8];
                let mut at = 0;
                for note in 0..128 {
                    if active.iter().any(|ch| ch[note]) && at < 8 {
                        chord[at] = 440. * 2f32.powf((note as f32 - 69.) / 12.);
                        at += 1;
                    }
                }
                events.push(((seconds * sample_rate as f64) as usize, chord));
            }
        }
        Ok(Self {
            events,
            length: ((seconds * sample_rate as f64) as usize).max(1),
        })
    }
    pub fn frequencies_at(&self, sample: usize) -> [f32; 8] {
        let at = sample % self.length;
        let index = self.events.partition_point(|e| e.0 <= at).saturating_sub(1);
        self.events[index].1
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tempo_map_and_note_off_are_sample_accurate() {
        let bytes=b"MThd\0\0\0\x06\0\0\0\x01\x01\xe0MTrk\0\0\0\x1b\0\x90\x45\x64\x83\x60\xff\x51\x03\x0f\x42\x40\x83\x60\x80\x45\0\x83\x60\xff\x2f\0";
        let midi = Timeline::parse(bytes, 48000.).unwrap();
        assert_eq!(midi.frequencies_at(23999)[0], 440.);
        assert_eq!(midi.frequencies_at(71999)[0], 440.);
        assert_eq!(midi.frequencies_at(72000)[0], 0.);
    }
}
