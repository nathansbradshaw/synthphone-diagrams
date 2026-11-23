//! Small reproducible browser QA inputs. Run with -- /absolute/output/directory.
use std::{f32::consts::TAU, fs, path::PathBuf};
fn vlq(mut n: u32, out: &mut Vec<u8>) {
    let mut bytes = [0; 4];
    let mut at = 3;
    bytes[at] = (n & 127) as u8;
    n >>= 7;
    while n > 0 {
        at -= 1;
        bytes[at] = (n as u8 & 127) | 128;
        n >>= 7;
    }
    out.extend_from_slice(&bytes[at..]);
}
fn main() {
    let dir = PathBuf::from(std::env::args().nth(1).expect("output directory"));
    fs::create_dir_all(&dir).unwrap();
    let mut pcm = Vec::with_capacity(48000 * 16 * 2);
    for i in 0..48000 * 16 {
        let t = i as f32 / 48000.;
        let f = [220., 261.6256, 329.6276, 293.6648][(t / 4.) as usize];
        let p = TAU * f * t;
        let x = (p.sin() * 0.22 + (2. * p).sin() * 0.07 + (3. * p).sin() * 0.035)
            * (0.8 + 0.2 * (t * TAU / 2.).sin());
        pcm.extend_from_slice(&((x * i16::MAX as f32) as i16).to_le_bytes());
    }
    let mut wav = b"RIFF".to_vec();
    wav.extend((36 + pcm.len() as u32).to_le_bytes());
    wav.extend(b"WAVEfmt ");
    wav.extend(16u32.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(48000u32.to_le_bytes());
    wav.extend(96000u32.to_le_bytes());
    wav.extend(2u16.to_le_bytes());
    wav.extend(16u16.to_le_bytes());
    wav.extend(b"data");
    wav.extend((pcm.len() as u32).to_le_bytes());
    wav.extend(pcm);
    fs::write(dir.join("bench-tone.wav"), wav).unwrap();
    let mut events = vec![0, 0xff, 0x51, 3, 7, 0xa1, 0x20];
    let mut delay = 480;
    for chord in [
        vec![60, 61, 62, 63, 64, 65, 66, 67],
        vec![69],
        vec![60, 64, 67],
        vec![72],
        vec![57, 60, 64],
        vec![65],
        vec![62, 65, 69],
        vec![67],
    ] {
        for (j, &key) in chord.iter().enumerate() {
            vlq(if j == 0 { delay } else { 0 }, &mut events);
            events.extend([0x90, key, 100]);
        }
        for (j, &key) in chord.iter().enumerate() {
            vlq(if j == 0 { 720 } else { 0 }, &mut events);
            events.extend([0x80, key, 0]);
        }
        delay = 240;
    }
    vlq(480 * 15, &mut events);
    events.extend([0xff, 0x2f, 0]);
    let mut midi = b"MThd\0\0\0\x06\0\0\0\x01\x01\xe0MTrk".to_vec();
    midi.extend((events.len() as u32).to_be_bytes());
    midi.extend(events);
    fs::write(dir.join("bench-chords.mid"), midi).unwrap();
    println!(
        "Wrote deterministic WAV and eight-note MIDI fixture to {}",
        dir.display()
    );
}
