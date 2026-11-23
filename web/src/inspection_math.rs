//! Calibrated axes and bounded inspection windows, independent of the browser.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Axis {
    pub lo: f64,
    pub hi: f64,
    pub log: bool,
}
impl Axis {
    /// Eight vertical divisions; a positive position moves the zero line up.
    pub fn waveform(sensitivity: f64, position: f64) -> Self {
        Self::linear(
            (-4. - position) * sensitivity,
            (4. - position) * sensitivity,
        )
    }
    pub fn linear(lo: f64, hi: f64) -> Self {
        Self { lo, hi, log: false }
    }
    pub fn logarithmic(lo: f64, hi: f64) -> Self {
        Self { lo, hi, log: true }
    }
    pub fn at(self, t: f64) -> f64 {
        if self.log {
            self.lo * (self.hi / self.lo).powf(t)
        } else {
            self.lo + (self.hi - self.lo) * t
        }
    }
    pub fn fraction(self, value: f64) -> f64 {
        if self.log {
            (value / self.lo).ln() / (self.hi / self.lo).ln()
        } else {
            (value - self.lo) / (self.hi - self.lo)
        }
    }
    pub fn slice(self, view: View) -> Self {
        Self {
            lo: self.at(view.start()),
            hi: self.at(view.end()),
            ..self
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct View {
    pub zoom: f64,
    pub center: f64,
}
impl Default for View {
    fn default() -> Self {
        Self {
            zoom: 1.,
            center: 0.5,
        }
    }
}
impl View {
    pub fn start(self) -> f64 {
        self.center - 0.5 / self.zoom
    }
    pub fn end(self) -> f64 {
        self.center + 0.5 / self.zoom
    }
    pub fn zoom_by(&mut self, factor: f64) {
        self.zoom = (self.zoom * factor).clamp(1., 32.);
        self.pan(0.);
    }
    pub fn pan(&mut self, step: f64) {
        self.center = (self.center + step / self.zoom).clamp(0.5 / self.zoom, 1. - 0.5 / self.zoom);
    }
}
pub fn period_hz(seconds: f64) -> Option<f64> {
    (seconds.is_finite() && seconds.abs() > 1e-9).then(|| 1. / seconds.abs())
}
/// Find a visible, finite trace vertex. Ordinary plots preserve the nearest
/// sample/bin on X; X–Y plots use distance in screen pixels instead.
pub fn nearest_vertex(
    points: &[[f64; 2]],
    pointer: [f64; 2],
    view: View,
    size: [f64; 2],
    xy: bool,
    direction: i8,
) -> Option<(usize, f64)> {
    points
        .iter()
        .enumerate()
        .filter_map(|(index, p)| {
            let x = (p[0] - view.start()) * view.zoom;
            if !x.is_finite()
                || !p[1].is_finite()
                || !(0. ..=1.).contains(&x)
                || !(0. ..=1.).contains(&p[1])
            {
                return None;
            }
            let dx = x - pointer[0];
            if direction != 0 && dx * direction as f64 <= 1e-10 {
                return None;
            }
            let dy = (p[1] - pointer[1]) * size[1];
            Some((index, dx.abs(), dx * dx * size[0] * size[0] + dy * dy))
        })
        .min_by(|a, b| {
            if xy {
                a.2.total_cmp(&b.2)
            } else {
                a.1.total_cmp(&b.1).then(a.2.total_cmp(&b.2))
            }
        })
        .map(|(index, _, distance)| (index, distance))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn waveform_position_does_not_change_measured_signal_level() {
        // The default CH1 zero line is at 33.125% from the top; CH2 at 66.875%.
        assert!(Axis::waveform(0.2, 1.35).at(1. - 0.33125).abs() < 1e-12);
        assert!(Axis::waveform(0.2, -1.35).at(1. - 0.66875).abs() < 1e-12);
        // A 0.2 FS sample sits exactly one division above either zero line.
        assert!((Axis::waveform(0.2, 1.35).at(1. - 0.20625) - 0.2).abs() < 1e-12);
        assert!((Axis::waveform(0.2, -1.35).at(1. - 0.54375) - 0.2).abs() < 1e-12);
    }
    #[test]
    fn snap_uses_real_vertices_and_skips_clipped_or_invalid_samples() {
        let points = [[0.2, 0.9], [0.4, 0.3], [0.6, 1.2], [f64::NAN, 0.5]];
        let (i, _) = nearest_vertex(
            &points,
            [0.43, 0.95],
            View::default(),
            [100., 100.],
            false,
            0,
        )
        .unwrap();
        assert_eq!(i, 1); // nearest X, not a fabricated point under the pointer
        let view = View {
            zoom: 2.,
            center: 0.5,
        };
        assert_eq!(
            nearest_vertex(&points, [0., 0.5], view, [100., 100.], false, 0)
                .unwrap()
                .0,
            1
        );
        assert!(nearest_vertex(&[], [0.5, 0.5], view, [100., 100.], false, 0).is_none());
    }
    #[test]
    fn xy_snap_uses_two_dimensional_distance_and_arrows_advance_samples() {
        let points = [[0.2, 0.2], [0.4, 0.9], [0.6, 0.3]];
        assert_eq!(
            nearest_vertex(
                &points,
                [0.39, 0.25],
                View::default(),
                [100., 100.],
                true,
                0
            )
            .unwrap()
            .0,
            0
        );
        assert_eq!(
            nearest_vertex(&points, [0.4, 0.9], View::default(), [100., 100.], false, 1)
                .unwrap()
                .0,
            2
        );
        assert_eq!(
            nearest_vertex(
                &points,
                [0.4, 0.9],
                View::default(),
                [100., 100.],
                false,
                -1
            )
            .unwrap()
            .0,
            0
        );
    }
    #[test]
    fn calibrated_linear_and_log_axes_round_trip() {
        for axis in [
            Axis::linear(-0.8, 0.8),
            Axis::linear(2., 2.010),
            Axis::logarithmic(20., 24000.),
        ] {
            for t in [0., 0.25, 0.5, 0.75, 1.] {
                assert!((axis.fraction(axis.at(t)) - t).abs() < 1e-10);
            }
        }
    }
    #[test]
    fn zoom_pan_never_leaves_the_capture() {
        let mut v = View::default();
        v.zoom_by(8.);
        v.pan(-100.);
        assert_eq!(v.start(), 0.);
        v.pan(100.);
        assert_eq!(v.end(), 1.);
        v.zoom_by(100.);
        assert_eq!(v.zoom, 32.);
        v.zoom_by(0.001);
        assert_eq!(v.start(), 0.);
        assert_eq!(v.end(), 1.);
    }
    #[test]
    fn log_zoom_and_linked_time_use_real_units() {
        let mut v = View::default();
        v.zoom_by(2.);
        let f = Axis::logarithmic(20., 20000.).slice(v);
        assert!((f.hi / f.lo - 1000f64.sqrt()).abs() < 1e-9);
        let wave = Axis::linear(4.990, 5.);
        let history = Axis::linear(0., 5.);
        assert!((history.at(history.fraction(wave.at(0.5))) - 4.995).abs() < 1e-10);
        assert_eq!(period_hz(0.), None);
        assert_eq!(period_hz(0.005), Some(200.));
    }
}
