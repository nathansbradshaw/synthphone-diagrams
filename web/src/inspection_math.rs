//! Calibrated axes and bounded inspection windows, independent of the browser.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Axis {
    pub lo: f64,
    pub hi: f64,
    pub log: bool,
}
impl Axis {
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
#[cfg(test)]
mod tests {
    use super::*;
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
