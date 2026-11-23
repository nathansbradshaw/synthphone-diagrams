// Score metadata is consumed only by the browser UI, not the raw audio module.
#![cfg_attr(
    not(all(target_arch = "wasm32", feature = "browser")),
    allow(dead_code)
)]
use midly::{MetaMessage, MidiMessage, Smf, Timing, TrackEventKind};
use std::collections::BTreeMap;
#[derive(Clone, Debug)]
pub struct Note {
    pub start: f64,
    pub end: f64,
    pub key: u8,
    pub track: usize,
    pub channel: u8,
}
pub struct Timeline {
    events: Vec<(usize, [f32; 8], f32)>,
    length: usize,
    sample_rate: f32,
    pub notes: Vec<Note>,
    pub tracks: Vec<String>,
    pub time_signature: (u8, u8),
}
impl Timeline {
    pub fn parse(bytes: &[u8], sample_rate: f32) -> Result<Self, String> {
        let smf = Smf::parse(bytes).map_err(|e| e.to_string())?;
        if smf.header.format == midly::Format::Sequential {
            return Err(
                "Format 2 contains independent sequences. Export MIDI format 0 or 1.".into(),
            );
        }
        let mut all = Vec::new();
        let mut tracks = Vec::new();
        for (track_index, track) in smf.tracks.iter().enumerate() {
            let mut tick = 0u64;
            let mut name = format!("Track {}", track_index + 1);
            for event in track {
                tick += event.delta.as_int() as u64;
                if let TrackEventKind::Meta(MetaMessage::TrackName(n)) = event.kind {
                    name = String::from_utf8_lossy(n).into_owned();
                }
                all.push((tick, track_index, event.kind));
            }
            tracks.push(name);
        }
        all.sort_by_key(|e| e.0);
        let mut events = vec![(0, [0.; 8], 0.)];
        let (mut previous, mut seconds, mut tempo) = (0u64, 0f64, 500000.);
        let mut signature = (4, 4);
        let mut active: BTreeMap<(usize, u8, u8), f64> = BTreeMap::new();
        let mut notes = Vec::new();
        for (tick, track, kind) in all {
            seconds += match smf.header.timing {
                Timing::Metrical(t) => (tick - previous) as f64 * tempo / (t.as_int() as f64 * 1e6),
                Timing::Timecode(fps, sub) => {
                    (tick - previous) as f64 / (fps.as_f32() as f64 * sub as f64)
                }
            };
            previous = tick;
            match kind {
                TrackEventKind::Meta(MetaMessage::Tempo(t)) => tempo = t.as_int() as f64,
                TrackEventKind::Meta(MetaMessage::TimeSignature(n, d, ..)) => {
                    signature = (n, 1u8.checked_shl(d as u32).unwrap_or(4))
                }
                TrackEventKind::Midi { channel, message } => {
                    let channel = channel.as_int();
                    let key = match message {
                        MidiMessage::NoteOn { key, .. } | MidiMessage::NoteOff { key, .. } => {
                            key.as_int()
                        }
                        _ => continue,
                    };
                    let id = (track, channel, key);
                    if let Some(start) = active.remove(&id) {
                        notes.push(Note {
                            start,
                            end: seconds,
                            key,
                            track,
                            channel,
                        });
                    }
                    if matches!(message,MidiMessage::NoteOn{vel,..} if vel.as_int()>0) {
                        active.insert(id, seconds);
                    }
                    let mut held = [false; 128];
                    for (_, _, note) in active.keys() {
                        held[*note as usize] = true;
                    }
                    let mut chord = [0.; 8];
                    let mut count = 0;
                    let mut target = 0.;
                    for (key, held) in held.iter().enumerate() {
                        if *held {
                            target = note_hz(key as u8);
                            if count < 8 {
                                chord[count] = target;
                                count += 1;
                            }
                        }
                    }
                    events.push((
                        (seconds * sample_rate as f64).round() as usize,
                        chord,
                        target,
                    ));
                }
                _ => {}
            }
        }
        for ((track, channel, key), start) in active {
            notes.push(Note {
                start,
                end: seconds.max(start + 0.001),
                key,
                track,
                channel,
            });
        }
        notes.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.key.cmp(&b.key)));
        Ok(Self {
            events,
            length: ((seconds * sample_rate as f64).round() as usize).max(1),
            sample_rate,
            notes,
            tracks,
            time_signature: signature,
        })
    }
    pub fn state_at(&self, sample: usize) -> ([f32; 8], f32) {
        let at = sample % self.length;
        let index = self.events.partition_point(|e| e.0 <= at).saturating_sub(1);
        (self.events[index].1, self.events[index].2)
    }
    pub fn duration(&self) -> f64 {
        self.length as f64 / self.sample_rate as f64
    }
    pub fn wrapped_seconds(&self, seconds: f64) -> f64 {
        seconds.rem_euclid(self.duration())
    }
}
pub fn note_hz(note: u8) -> f32 {
    440. * 2f32.powf((note as f32 - 69.) / 12.)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn vlq(mut value: u32) -> Vec<u8> {
        let mut b = vec![(value & 127) as u8];
        value >>= 7;
        while value > 0 {
            b.push(((value & 127) | 128) as u8);
            value >>= 7;
        }
        b.reverse();
        b
    }
    fn file(events: &[u8]) -> Vec<u8> {
        let mut b = b"MThd\0\0\0\x06\0\0\0\x01\x01\xe0MTrk".to_vec();
        b.extend((events.len() as u32).to_be_bytes());
        b.extend(events);
        b
    }
    #[test]
    fn tempo_changes_and_note_off_sample_accurate() {
        let bytes=b"MThd\0\0\0\x06\0\0\0\x01\x01\xe0MTrk\0\0\0\x1b\0\x90\x45\x64\x83\x60\xff\x51\x03\x0f\x42\x40\x83\x60\x80\x45\0\x83\x60\xff\x2f\0";
        let m = Timeline::parse(bytes, 48000.).unwrap();
        assert_eq!(m.state_at(71999).1, 440.);
        assert_eq!(m.state_at(72000).1, 0.);
        assert_eq!(m.notes.len(), 1);
        assert_eq!(m.notes[0].end, 1.5);
    }
    #[test]
    fn eight_note_chord_and_top_note_target_survive_single_track() {
        let mut e = Vec::new();
        for note in 60..68 {
            e.extend([0, 0x90, note, 100]);
        }
        e.extend(vlq(480));
        e.extend([0x80, 60, 0]);
        for note in 61..68 {
            e.extend([0, 0x80, note, 0]);
        }
        e.extend(vlq(480));
        e.extend([0xff, 0x2f, 0]);
        let m = Timeline::parse(&file(&e), 48000.).unwrap();
        let (chord, top) = m.state_at(1000);
        assert!(chord.iter().all(|x| *x > 0.));
        assert_eq!(top, note_hz(67));
        assert_eq!(m.notes.len(), 8);
        assert_eq!(m.state_at(24000).1, 0.);
    }
    #[test]
    fn channels_do_not_release_one_another_and_zero_velocity_is_off() {
        let e = [
            0, 0x90, 69, 100, 0, 0x91, 69, 100, 0x81, 0x70, 0x90, 69, 0, 0x81, 0x70, 0x81, 69, 0,
            0x81, 0x70, 0xff, 0x2f, 0,
        ];
        let m = Timeline::parse(&file(&e), 48000.).unwrap();
        assert_eq!(m.state_at(15000).1, 440.);
        assert_eq!(m.state_at(24000).1, 0.);
        assert_eq!(m.notes.len(), 2);
    }
}
impl Timeline {
    /// Compact, already-resolved events. The audio worklet never parses SMF or
    /// builds note/track metadata; browser Rust performs that work once.
    pub fn encode(&self) -> Vec<u8> {
        let mut data = Vec::with_capacity(24 + self.events.len() * 44);
        data.extend(b"DSPMIDI1");
        data.extend(self.sample_rate.to_le_bytes());
        data.extend((self.length as u64).to_le_bytes());
        data.extend((self.events.len() as u32).to_le_bytes());
        for (position, chord, target) in &self.events {
            data.extend((*position as u64).to_le_bytes());
            for hz in chord {
                data.extend(hz.to_le_bytes());
            }
            data.extend(target.to_le_bytes());
        }
        data
    }
    pub fn decode(data: &[u8], sample_rate: f32) -> Result<Self, String> {
        if data.len() < 24 || &data[..8] != b"DSPMIDI1" {
            return Err("Invalid prepared MIDI timeline.".into());
        }
        let source_rate = f32::from_le_bytes(data[8..12].try_into().unwrap());
        let length = u64::from_le_bytes(data[12..20].try_into().unwrap());
        let count = u32::from_le_bytes(data[20..24].try_into().unwrap()) as usize;
        if !source_rate.is_finite()
            || source_rate <= 0.
            || count > 500_000
            || data.len() != 24 + count * 44
        {
            return Err("Prepared MIDI timeline exceeds its bounds.".into());
        }
        let convert = |value: u64| -> Result<usize, String> {
            let samples = (value as f64 * sample_rate as f64 / source_rate as f64).round();
            if samples > usize::MAX as f64 {
                return Err("MIDI timeline is too long for this browser.".into());
            }
            Ok(samples as usize)
        };
        let mut events = Vec::with_capacity(count);
        let mut previous = 0;
        for record in data[24..].chunks_exact(44) {
            let at = convert(u64::from_le_bytes(record[..8].try_into().unwrap()))?;
            if at < previous {
                return Err("Unsorted MIDI timeline.".into());
            }
            previous = at;
            let mut chord = [0.; 8];
            for (i, hz) in chord.iter_mut().enumerate() {
                *hz = f32::from_le_bytes(record[8 + i * 4..12 + i * 4].try_into().unwrap());
            }
            let target = f32::from_le_bytes(record[40..44].try_into().unwrap());
            if chord
                .iter()
                .chain(std::iter::once(&target))
                .any(|f| !f.is_finite() || *f < 0.)
            {
                return Err("Invalid MIDI frequency.".into());
            }
            events.push((at, chord, target));
        }
        if events.is_empty() {
            events.push((0, [0.; 8], 0.));
        }
        Ok(Self {
            events,
            length: convert(length)?.max(1),
            sample_rate,
            notes: Vec::new(),
            tracks: Vec::new(),
            time_signature: (4, 4),
        })
    }
}
#[cfg(test)]
mod prepared_tests {
    use super::*;
    #[test]
    fn prepared_timeline_round_trips_and_resamples() {
        let bytes=b"MThd\0\0\0\x06\0\0\0\x01\x01\xe0MTrk\0\0\0\x1b\0\x90\x45\x64\x83\x60\xff\x51\x03\x0f\x42\x40\x83\x60\x80\x45\0\x83\x60\xff\x2f\0";
        let original = Timeline::parse(bytes, 48000.).unwrap();
        let copy = Timeline::decode(&original.encode(), 44100.).unwrap();
        assert_eq!(copy.state_at(66149).1, 440.);
        assert_eq!(copy.state_at(66150).1, 0.);
        assert_eq!(copy.duration(), original.duration());
        assert!(Timeline::decode(b"bad", 48000.).is_err());
    }
}
