/// The DSP library orders keys around the circle of fifths, rather than chromatically.
pub fn library_key(pitch_class: usize, minor: bool) -> i8 {
    const MAJOR: [i8; 12] = [0, 7, 2, 10, 4, 8, 6, 1, 11, 3, 9, 5];
    const MINOR: [i8; 12] = [20, 16, 18, 23, 13, 21, 15, 19, 17, 12, 22, 14];
    if minor {
        MINOR[pitch_class % 12]
    } else {
        MAJOR[pitch_class % 12]
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_key_matches_the_library_root() {
        for minor in [false, true] {
            for (pc, expected) in [
                "C", "C#", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B",
            ]
            .iter()
            .enumerate()
            {
                let root =
                    synthphone_e_vocal_dsp::audio::keys::get_key_name(library_key(pc, minor));
                assert!(
                    root == *expected || (root == "G#" && *expected == "Ab"),
                    "{root} vs {expected}"
                );
            }
        }
    }
}
