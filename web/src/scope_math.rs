//! Display math stays independent of browser I/O so calibration and triggering are testable.
pub fn trigger_index(samples: &[f32], count: usize, level: f32, rising: bool) -> Option<usize> {
    if count == 0 || count > samples.len() {
        return None;
    }
    samples
        .windows(2)
        .take(samples.len() - count)
        .position(|s| {
            if rising {
                s[0] < level && s[1] >= level
            } else {
                s[0] > level && s[1] <= level
            }
        })
}
pub fn decimate_extrema(samples: &[f32], pixels: usize) -> Vec<(f32, f32)> {
    if pixels == 0 || samples.is_empty() {
        return Vec::new();
    }
    let n = pixels.min(samples.len());
    (0..n)
        .map(|i| {
            let a = i * samples.len() / n;
            let b = ((i + 1) * samples.len() / n).max(a + 1);
            samples[a..b]
                .iter()
                .copied()
                .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), v| {
                    (lo.min(v), hi.max(v))
                })
        })
        .collect()
}
pub fn note_name(hz: f32) -> String {
    if !hz.is_finite() || hz < 20. {
        return "—".into();
    }
    let midi = (69. + 12. * (hz / 440.).log2()).round() as i32;
    let names = [
        "C", "C♯", "D", "D♯", "E", "F", "F♯", "G", "G♯", "A", "A♯", "B",
    ];
    format!(
        "{}{}",
        names[midi.rem_euclid(12) as usize],
        midi.div_euclid(12) - 1
    )
}
pub fn midi_staff_step(key: u8) -> i32 {
    let steps = [0, 0, 1, 1, 2, 3, 3, 4, 4, 5, 5, 6];
    (key / 12) as i32 * 7 + steps[(key % 12) as usize]
}
#[cfg_attr(test, allow(dead_code))]
pub fn analysis_mask(view: u8) -> u8 {
    match view {
        1 | 6 => 7,
        2 => 1,
        3 => 4,
        4 => 3,
        7 => 12,
        _ => 0,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rising_falling_and_full_window_guard() {
        assert_eq!(
            trigger_index(&[-1., -0.5, 0.5, 1., 0.], 2, 0., true),
            Some(1)
        );
        assert_eq!(trigger_index(&[1., 0.5, -0.5, -1.], 2, 0., false), Some(1));
        assert_eq!(trigger_index(&[0.; 4], 5, 0., true), None);
    }
    #[test]
    fn decimation_preserves_short_transients() {
        let mut s = vec![0.; 1000];
        s[407] = 0.95;
        s[408] = -0.8;
        let e = decimate_extrema(&s, 100);
        assert!(e.iter().any(|x| x.1 == 0.95 && x.0 == -0.8));
    }
    #[test]
    fn concert_pitch_and_staff_steps() {
        assert_eq!(note_name(440.), "A4");
        assert_eq!(note_name(220.), "A3");
        assert_eq!(midi_staff_step(60), 35);
        assert_eq!(midi_staff_step(64), 37);
        assert_eq!(midi_staff_step(61), 35);
    }
}
