/// MIDI file timeline: maps playback sample-position → active note frequencies
/// (per track, for multi-voice harmony), plus note/timing data for the staff
/// visualization.
use std::collections::HashMap;
use std::path::Path;

/// Max simultaneous voice parts (matches `MusicalSettings::midi_frequencies`).
pub const MAX_TRACKS: usize = 8;

/// A fully resolved note (paired NoteOn → NoteOff) for visualization.
#[derive(Debug, Clone, Copy)]
pub struct MidiNote {
    pub start_tick: u64,
    pub end_tick: u64,
    pub midi_note: u8,
    pub track: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct TempoChange {
    pub at_tick: u64,
    pub micros_per_beat: u32,
}

pub struct MidiTimeline {
    /// One DSP event list per voice part: sorted (sample_offset, frequency_hz).
    /// 0.0 = rest. Track index = voice part, remapped to 0..track_count.
    track_events: [Vec<(usize, f32)>; MAX_TRACKS],
    pub track_count: usize,

    // Visualization metadata
    pub notes: Vec<MidiNote>,
    pub ticks_per_beat: u64,
    pub tempo_map: Vec<TempoChange>,
    pub time_sig: (u8, u8),
    #[allow(dead_code)]
    pub total_ticks: u64,
    sample_rate: u32,
}

impl MidiTimeline {
    pub fn load(
        path: &Path,
        sample_rate: u32,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let data = std::fs::read(path)?;
        let smf = midly::Smf::parse(&data)?;

        let ticks_per_beat = match smf.header.timing {
            midly::Timing::Metrical(t) => t.as_int() as u64,
            midly::Timing::Timecode(fps, sub) => (fps.as_f32() * sub as f32) as u64,
        };

        // Collect every event with its absolute tick and originating track.
        let mut all_events: Vec<(u64, usize, midly::TrackEventKind<'_>)> = Vec::new();
        for (track_idx, track) in smf.tracks.iter().enumerate() {
            let mut abs_tick: u64 = 0;
            for ev in track {
                abs_tick += ev.delta.as_int() as u64;
                all_events.push((abs_tick, track_idx, ev.kind));
            }
        }
        all_events.sort_by_key(|(t, ti, _)| (*t, *ti));

        let mut tempo_map: Vec<TempoChange> = Vec::new();
        let mut time_sig = (4u8, 4u8);
        let mut total_ticks: u64 = 0;

        for (tick, _, kind) in &all_events {
            total_ticks = total_ticks.max(*tick);
            if let midly::TrackEventKind::Meta(meta) = kind {
                match meta {
                    midly::MetaMessage::Tempo(t) => {
                        tempo_map.push(TempoChange { at_tick: *tick, micros_per_beat: t.as_int() });
                    }
                    midly::MetaMessage::TimeSignature(num, den_pow, _, _) => {
                        time_sig = (*num, 1u8 << den_pow);
                    }
                    _ => {}
                }
            }
        }
        if tempo_map.is_empty() || tempo_map[0].at_tick > 0 {
            tempo_map.insert(0, TempoChange { at_tick: 0, micros_per_beat: 500_000 });
        }
        tempo_map.sort_by_key(|t| t.at_tick);

        let tick_to_sample = |tick: u64| -> usize {
            let mut acc_samples: f64 = 0.0;
            let mut prev_tick: u64 = 0;
            let mut us_per_beat: u64 = 500_000;
            for tc in &tempo_map {
                if tc.at_tick >= tick { break; }
                let seg = (tc.at_tick - prev_tick) as f64;
                acc_samples += seg * us_per_beat as f64 / (ticks_per_beat as f64 * 1_000_000.0)
                    * sample_rate as f64;
                prev_tick = tc.at_tick;
                us_per_beat = tc.micros_per_beat as u64;
            }
            let seg = (tick - prev_tick) as f64;
            acc_samples += seg * us_per_beat as f64 / (ticks_per_beat as f64 * 1_000_000.0)
                * sample_rate as f64;
            acc_samples as usize
        };

        // Remap original track indices (which may skip meta-only tracks) to
        // sequential voice-part indices 0..track_count, capped at MAX_TRACKS.
        let mut midi_tracks: Vec<usize> = all_events
            .iter()
            .filter(|(_, _, k)| matches!(k, midly::TrackEventKind::Midi { .. }))
            .map(|(_, ti, _)| *ti)
            .collect();
        midi_tracks.sort_unstable();
        midi_tracks.dedup();
        midi_tracks.truncate(MAX_TRACKS);
        let track_count = midi_tracks.len();
        let track_remap: HashMap<usize, usize> = midi_tracks
            .iter()
            .enumerate()
            .map(|(new, &old)| (old, new))
            .collect();

        let mut track_events: [Vec<(usize, f32)>; MAX_TRACKS] = Default::default();
        let mut notes: Vec<MidiNote> = Vec::new();
        // Per remapped-track active-note state: note -> start_tick.
        let mut active: Vec<HashMap<u8, u64>> = vec![HashMap::new(); MAX_TRACKS];

        for (tick, orig_track, kind) in &all_events {
            let Some(&track) = track_remap.get(orig_track) else { continue };
            if let midly::TrackEventKind::Midi { message, .. } = kind {
                match message {
                    midly::MidiMessage::NoteOn { key, vel } if vel.as_int() > 0 => {
                        let note = key.as_int();
                        active[track].insert(note, *tick);
                        let top = top_active_hz(&active[track]);
                        track_events[track].push((tick_to_sample(*tick), top));
                    }
                    midly::MidiMessage::NoteOn { key, .. }
                    | midly::MidiMessage::NoteOff { key, .. } => {
                        let note = key.as_int();
                        if let Some(start_tick) = active[track].remove(&note) {
                            notes.push(MidiNote { start_tick, end_tick: *tick, midi_note: note, track });
                        }
                        let top = top_active_hz(&active[track]);
                        track_events[track].push((tick_to_sample(*tick), top));
                    }
                    _ => {}
                }
            }
        }

        // Close any notes that never received a NoteOff.
        for (track, notes_active) in active.iter().enumerate() {
            for (&note, &start_tick) in notes_active {
                notes.push(MidiNote { start_tick, end_tick: total_ticks, midi_note: note, track });
            }
        }
        notes.sort_by_key(|n| n.start_tick);
        for evs in track_events.iter_mut() {
            evs.sort_by_key(|(s, _)| *s);
        }

        Ok(Self {
            track_events,
            track_count,
            notes,
            ticks_per_beat,
            tempo_map,
            time_sig,
            total_ticks,
            sample_rate,
        })
    }

    /// Per-voice-part active frequency at `sample_pos`. Index = track, 0.0 = rest.
    /// Feed directly into `MusicalSettings::midi_frequencies` for multi-voice harmony.
    pub fn freqs_at(&self, sample_pos: usize) -> [f32; MAX_TRACKS] {
        let mut out = [0.0f32; MAX_TRACKS];
        for (t, evs) in self.track_events.iter().enumerate().take(self.track_count) {
            if evs.is_empty() { continue; }
            let idx = evs.partition_point(|(s, _)| *s <= sample_pos);
            if idx > 0 { out[t] = evs[idx - 1].1; }
        }
        out
    }

    /// Highest active frequency across all voice parts (used to drive a single
    /// MIDI-following carrier, e.g. Vocoder Harmony mode).
    pub fn freq_at(&self, sample_pos: usize) -> f32 {
        self.freqs_at(sample_pos).into_iter().fold(0.0, f32::max)
    }

    /// Convert a playback sample position to a MIDI tick, for the staff playhead.
    pub fn sample_to_tick(&self, sample_pos: usize) -> u64 {
        let mut acc_samples: f64 = 0.0;
        let mut prev_tick: u64 = 0;
        let mut us_per_beat: u64 = 500_000;
        for tc in &self.tempo_map {
            let seg_ticks = (tc.at_tick - prev_tick) as f64;
            let seg_samples = seg_ticks * us_per_beat as f64 / (self.ticks_per_beat as f64 * 1_000_000.0)
                * self.sample_rate as f64;
            if acc_samples + seg_samples > sample_pos as f64 {
                let delta = sample_pos as f64 - acc_samples;
                let ticks_per_sample = self.ticks_per_beat as f64 * 1_000_000.0
                    / (self.sample_rate as f64 * us_per_beat as f64);
                return prev_tick + (delta * ticks_per_sample) as u64;
            }
            acc_samples += seg_samples;
            prev_tick = tc.at_tick;
            us_per_beat = tc.micros_per_beat as u64;
        }
        let delta = sample_pos as f64 - acc_samples;
        let ticks_per_sample = self.ticks_per_beat as f64 * 1_000_000.0
            / (self.sample_rate as f64 * us_per_beat as f64);
        prev_tick + (delta * ticks_per_sample).max(0.0) as u64
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.notes.is_empty()
    }
}

fn top_active_hz(active: &HashMap<u8, u64>) -> f32 {
    active.keys().copied().max().map(midi_note_to_hz).unwrap_or(0.0)
}

fn midi_note_to_hz(note: u8) -> f32 {
    440.0 * 2.0_f32.powf((note as f32 - 69.0) / 12.0)
}
