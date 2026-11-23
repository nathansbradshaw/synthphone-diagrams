use super::inspection::{self, FREQUENCY, INPUT, PERIOD, Plot, TIME};
use super::*;
use crate::engine::*;
use crate::inspection_math::Axis;
use crate::scope_math::{decimate_extrema, midi_staff_step, note_name, trigger_index};
#[derive(Clone)]
pub(super) struct Settings {
    pub time: f64,
    pub sensitivity: [f64; 2],
    pub position: [f64; 2],
    pub coupling: [u8; 2],
    pub shown: [bool; 2],
    pub envelopes: [bool; 3],
    pub intensity: f64,
    pub focus: f64,
    pub illumination: f64,
    pub persistence: f64,
    pub trigger_mode: u8,
    pub trigger_source: usize,
    pub rising: bool,
    pub level: f32,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            time: 1.,
            sensitivity: [0.2; 2],
            position: [1.35, -1.35],
            coupling: [0, 0],
            shown: [true; 2],
            envelopes: [true; 3],
            intensity: 0.7,
            focus: 0.12,
            illumination: 0.45,
            persistence: 0.15,
            trigger_mode: 0,
            trigger_source: 0,
            rising: true,
            level: 0.,
        }
    }
}
impl Settings {
    pub fn read_controls(&mut self) {
        self.time = controls::read("timebase") as f64 * controls::read("fine-time") as f64;
        self.sensitivity = [
            controls::read("vertical") as f64 * controls::read("fine-a") as f64,
            controls::read("vertical-out") as f64 * controls::read("fine-b") as f64,
        ];
        self.position = [
            controls::read("position-a") as f64,
            controls::read("position-b") as f64,
        ];
        self.shown = [input("show-a").checked(), input("show-b").checked()];
        for (i, name) in ["coupling-a", "coupling-b"].iter().enumerate() {
            if let Some(e) = doc()
                .query_selector(&format!("input[name='{name}']:checked"))
                .unwrap()
            {
                self.coupling[i] = e
                    .unchecked_into::<HtmlInputElement>()
                    .value()
                    .parse()
                    .unwrap_or(0);
            }
        }
        self.envelopes = [
            input("env-before").checked(),
            input("env-ideal").checked(),
            input("env-actual").checked(),
        ];
        self.intensity = controls::read("intensity") as f64;
        self.focus = controls::read("focus") as f64;
        self.illumination = controls::read("illumination") as f64;
        self.persistence = controls::read("persistence") as f64;
        self.level = controls::read("trigger-level");
        let _ = el("scope").set_attribute(
            "style",
            &format!(
                "--focus-blur:{}px;--trace-opacity:{}",
                self.focus * 0.8,
                0.25 + self.intensity * 0.75
            ),
        );
        let _ = el("grid").set_attribute(
            "style",
            &format!("--grid-opacity:{}", 0.12 + self.illumination * 0.7),
        );
    }
}
#[derive(Clone, Copy)]
struct Pitch {
    time: f64,
    a: f32,
    b: f32,
    target: f32,
}
#[derive(Clone, Copy)]
pub(super) struct Area {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}
pub(super) struct Display {
    pub settings: Settings,
    history: VecDeque<Pitch>,
    waterfalls: Option<[HtmlCanvasElement; 2]>,
    peaks: [f32; 2],
    times: VecDeque<f64>,
    serial: f32,
    last_time: f64,
    last_view: u8,
    pub fresh: bool,
    pub frames_base: f64,
    pub fft_base: f64,
    last_frames: f32,
    last_fft: f32,
}
impl Default for Display {
    fn default() -> Self {
        Self {
            settings: Settings::default(),
            history: VecDeque::with_capacity(512),
            waterfalls: None,
            peaks: [1e-5; 2],
            times: VecDeque::with_capacity(512),
            serial: -1.,
            last_time: 0.,
            last_view: 255,
            fresh: false,
            frames_base: 0.,
            fft_base: 0.,
            last_frames: 0.,
            last_fft: 0.,
        }
    }
}
fn ctx(canvas: &HtmlCanvasElement) -> CanvasRenderingContext2d {
    canvas.get_context("2d").unwrap().unwrap().unchecked_into()
}
fn canvas(id: &str) -> HtmlCanvasElement {
    el(id).unchecked_into()
}
fn label(c: &CanvasRenderingContext2d, s: &str, x: f64, y: f64) {
    c.set_fill_style_str("#a3d9be");
    let _ = c.fill_text(s, x, y);
}
fn stroke(c: &CanvasRenderingContext2d, points: impl Iterator<Item = (f64, f64)>, ch: usize) {
    c.set_stroke_style_str(if ch == 0 { "#72ffe0" } else { "#a6ffb2" });
    c.set_line_width(1.65);
    c.begin_path();
    let mut first = true;
    for (x, y) in points {
        if first {
            c.move_to(x, y);
            first = false;
        } else {
            c.line_to(x, y);
        }
    }
    c.stroke();
}
fn line(c: &CanvasRenderingContext2d, x: f64, y: f64, x2: f64, y2: f64) {
    c.begin_path();
    c.move_to(x, y);
    c.line_to(x2, y2);
    c.stroke();
}
fn db(x: f32) -> f64 {
    20. * x.max(1e-8).log10() as f64
}
fn lattice(c: &CanvasRenderingContext2d, r: Area, columns: usize, rows: usize) {
    c.set_stroke_style_str("#47766b");
    c.set_line_width(0.55);
    c.begin_path();
    for col in 0..=columns {
        let x = r.x + col as f64 * r.w / columns as f64;
        c.move_to(x, r.y);
        c.line_to(x, r.y + r.h);
    }
    for row in 0..=rows {
        let y = r.y + row as f64 * r.h / rows as f64;
        c.move_to(r.x, y);
        c.line_to(r.x + r.w, y);
    }
    c.stroke();
}
impl Display {
    pub fn draw_aux(
        &self,
        s: &[f32; SNAPSHOT_LEN],
        live: bool,
        midi: Option<&crate::midi::Timeline>,
    ) {
        let sr = if s[META + 9] > 0. {
            s[META + 9]
        } else {
            48000.
        };
        let r = Area {
            x: 48.,
            y: 38.,
            w: 500.,
            h: 300.,
        };
        for i in 0..3 {
            inspection::start(i + 1);
            let c = ctx(&canvas(&format!("aux-{i}")));
            c.clear_rect(0., 0., 600., 400.);
            c.set_font("26px monospace");
            lattice(&c, r, 10, 6);
            if !live {
                label(&c, "POWER OFF", 228., 205.);
                continue;
            }
            match value(&format!("aux-view-{i}")) as u8 {
                1 => self.spectrum(&c, r, s, sr, true),
                2 => self.spectrograms(&c, r, sr),
                3 => self.pitch(&c, r),
                4 => self.envelopes(&c, r, s, sr),
                5 => draw_midi(&c, r, midi, s[META + 10] as f64, s[META + 6]),
                7 => self.cepstrum(&c, r, s, sr),
                _ => self.xy(&c, r, s),
            }
            inspection::finish(&c);
        }
    }
    pub fn reset(&mut self) {
        self.frames_base += self.last_frames as f64;
        self.fft_base += self.last_fft as f64;
        self.last_frames = 0.;
        self.last_fft = 0.;
        self.clear_history();
    }
    pub fn clear_history(&mut self) {
        inspection::reset_capture();
        self.history.clear();
        self.waterfalls = None;
        self.times.clear();
        self.serial = -1.;
        self.last_time = 0.;
        self.peaks = [1e-5; 2];
        self.last_view = 255;
    }
    pub fn consume(&mut self, s: &[f32; SNAPSHOT_LEN]) {
        let time = s[META + 10] as f64;
        if time < self.last_time || time - self.last_time > 0.5 {
            self.history.clear();
            self.waterfalls = None;
            self.times.clear();
        }
        self.last_time = time;
        self.last_frames = s[META + 7];
        self.last_fft = s[META + 8];
        self.fresh = true;
        if s[META + 7] == self.serial {
            return;
        }
        self.serial = s[META + 7];
        let mask = s[META + 14] as u8;
        if mask & 4 != 0 {
            if self.history.len() == 512 {
                self.history.pop_front();
            }
            self.history.push_back(Pitch {
                time,
                a: s[META],
                b: s[META + 1],
                target: if matches!(s[META + 13] as u8, 4 | 7) {
                    s[META + 6]
                } else {
                    0.
                },
            });
        }
        if mask & 1 != 0 {
            if self.waterfalls.is_none() {
                let a: HtmlCanvasElement = doc().create_element("canvas").unwrap().unchecked_into();
                a.set_width(512);
                a.set_height(128);
                let b: HtmlCanvasElement = doc().create_element("canvas").unwrap().unchecked_into();
                b.set_width(512);
                b.set_height(128);
                self.waterfalls = Some([a, b]);
            }
            let sr = s[META + 9].max(1.);
            for (channel, offset) in [FFT_A, FFT_B].into_iter().enumerate() {
                let image = &self.waterfalls.as_ref().unwrap()[channel];
                let c = ctx(image);
                let _ = c
                    .draw_image_with_html_canvas_element_and_sw_and_sh_and_dx_and_dy_and_dw_and_dh(
                        image, 1., 0., 511., 128., 0., 0., 511., 128.,
                    );
                self.peaks[channel] = (self.peaks[channel] * 0.98)
                    .max(s[offset..offset + 512].iter().copied().fold(1e-5, f32::max));
                let mut pixels = [0u8; 512];
                for y in 0..128 {
                    let hz = 20. * ((sr / 2.) / 20.).powf(1. - y as f32 / 127.);
                    let bin = (hz * 1024. / sr).clamp(1., 511.) as usize;
                    let level =
                        ((db(s[offset + bin] / self.peaks[channel]) + 70.) / 70.).clamp(0., 1.);
                    let p = y * 4;
                    pixels[p] = (25. + 110. * level) as u8;
                    pixels[p + 1] = (30. + 225. * level) as u8;
                    pixels[p + 2] = (26. + 150. * level) as u8;
                    pixels[p + 3] = 255;
                }
                let image_data = ImageData::new_with_u8_clamped_array_and_sh(
                    wasm_bindgen::Clamped(&pixels),
                    1,
                    128,
                )
                .unwrap();
                let _ = c.put_image_data(&image_data, 511., 0.);
            }
            if self.times.len() == 512 {
                self.times.pop_front();
            }
            self.times.push_back(time);
        }
    }
    pub fn draw(
        &mut self,
        s: &[f32; SNAPSHOT_LEN],
        view: u8,
        live: bool,
        playing: bool,
        frozen: bool,
        midi: Option<&crate::midi::Timeline>,
    ) -> bool {
        inspection::start(0);
        let c = ctx(&canvas("scope"));
        let g = ctx(&canvas("grid"));
        let r = Area {
            x: 62.,
            y: 79.,
            w: 876.,
            h: 637.,
        };
        let sr = if s[META + 9] > 0. {
            s[META + 9]
        } else {
            48000.
        };
        let changed = self.last_view != view;
        self.last_view = view;
        g.clear_rect(0., 0., 1000., 800.);
        lattice(&g, r, 10, 8);
        g.set_stroke_style_str("#548b79");
        g.set_line_width(1.0);
        line(&g, 62., 397.5, 938., 397.5);
        line(&g, 500., 79., 500., 716.);
        for i in 0..=50 {
            let x = 62. + r.w * i as f64 / 50.;
            line(&g, x, 393., x, 402.);
        }
        for i in 0..=40 {
            let y = 79. + r.h * i as f64 / 40.;
            line(&g, 496., y, 504., y);
        }
        let waiting = live
            && playing
            && !frozen
            && view == 0
            && self.settings.trigger_mode != 0
            && wave_edge(s, &self.settings, sr).is_none();
        if waiting {
            // Keep the actual previous acquisition while Normal/Single wait.
        } else if view != 0
            || changed
            || self.settings.persistence < 0.015
            || !live
            || !playing
            || frozen
        {
            c.clear_rect(0., 0., 1000., 800.);
        } else {
            let _ = c.set_global_composite_operation("destination-out");
            c.set_fill_style_str(&format!(
                "rgba(0,0,0,{})",
                (1. - self.settings.persistence * 0.94).max(0.05)
            ));
            c.fill_rect(0., 0., 1000., 800.);
            let _ = c.set_global_composite_operation("source-over");
        }
        c.set_font("24px monospace");
        c.set_line_width(1.5);
        c.set_shadow_color("#5cffcb");
        c.set_shadow_blur(self.settings.intensity * 3.);
        text(
            "view-label",
            [
                "WAVEFORM",
                "SPECTRUM",
                "SPECTROGRAM",
                "PITCH",
                "FORMANT ENVELOPES",
                "MIDI SCORE",
                "COMPARE",
                "CEPSTRUM",
                "X–Y",
            ][view.min(8) as usize],
        );
        text(
            "screen-state",
            if !live {
                "POWER OFF"
            } else if frozen {
                "HOLD"
            } else if !playing {
                "PAUSED"
            } else {
                "RUN"
            },
        );
        text(
            "input-level",
            &if live {
                format!("{:.1} dBFS", db(s[META + 2]))
            } else {
                "— dBFS".into()
            },
        );
        text(
            "output-level",
            &if live {
                format!("{:.1} dBFS", db(s[META + 3]))
            } else {
                "— dBFS".into()
            },
        );
        text(
            "stats",
            &format!(
                "{:.0} BLOCKS · ≈{:.0} FFTs",
                self.frames_base + s[META + 7] as f64,
                self.fft_base + s[META + 8] as f64
            ),
        );
        let total_fft = self.fft_base + s[META + 8] as f64;
        el("stats").set_attribute("title",&format!("Estimated FFT calls: {total_fft:.0}; approximately {:.0} FFT butterfly operations versus {:.0} naive DFT operations (102.4× fewer). Counts include estimated library FFT calls; no CPU percentage is claimed.",total_fft*1024.*10.,total_fft*1024.*1024.)).unwrap();
        text(
            "screen-scale",
            &format!(
                "{} ms/div · CH1 {} / CH2 {} FS/div",
                compact(self.settings.time),
                compact(self.settings.sensitivity[0]),
                compact(self.settings.sensitivity[1])
            ),
        );
        text(
            "measurement",
            &if live && s[META + 14] as u8 & 4 != 0 {
                format!("{} / {}", note_name(s[META]), note_name(s[META + 1]))
            } else {
                "—".into()
            },
        );
        let mut captured = false;
        if live {
            match view {
                0 => {
                    captured = self.wave(&c, r, s, sr, playing, frozen || !playing);
                }
                1 => self.spectrum(&c, r, s, sr, true),
                2 => self.spectrograms(&c, r, sr),
                3 => self.pitch(&c, r),
                4 => self.envelopes(&c, r, s, sr),
                5 => draw_midi(&c, r, midi, s[META + 10] as f64, s[META + 6]),
                6 => self.compare(&c, r, s, sr),
                7 => self.cepstrum(&c, r, s, sr),
                8 => self.xy(&c, r, s),
                _ => {}
            }
            c.set_shadow_blur(0.);
            if s[META + 4] >= 0.999 {
                label(&c, "CH1 CLIP", 80., 61.);
            }
            if s[META + 5] >= 0.999 {
                label(&c, "CH2 CLIP", 785., 61.);
            }
        } else {
            c.set_shadow_blur(0.);
            c.set_font("18px monospace");
            label(&c, "POWER ON TO ACQUIRE", 320., 396.);
        }
        self.fresh = false;
        if live {
            inspection::finish(&c);
        }
        captured
    }
    fn wave_plot(&self, r: Area, s: &[f32; SNAPSHOT_LEN], sr: f32, channel: Option<usize>) -> Plot {
        let count = (self.settings.time * 10. * sr as f64 / 1000.)
            .round()
            .clamp(2., 1024.) as usize;
        let start = wave_edge(s, &self.settings, sr).unwrap_or(WAVE - count);
        let end = s[META + 10] as f64;
        let x = Axis::linear(
            end - (WAVE - start) as f64 / sr as f64,
            end - (WAVE - start - count + usize::from(channel.is_some())) as f64 / sr as f64,
        );
        let axis = |ch: usize| {
            Axis::linear(
                (-4. + self.settings.position[ch]) * self.settings.sensitivity[ch],
                (4. + self.settings.position[ch]) * self.settings.sensitivity[ch],
            )
        };
        let y = if let Some(ch) = channel {
            [Some(axis(ch)), None]
        } else {
            [Some(axis(0)), Some(axis(1))]
        };
        Plot {
            area: r,
            domain: TIME,
            x,
            y,
            unit: "FS",
        }
    }
    fn frequency_plot(r: Area, sr: f32) -> Plot {
        Plot {
            area: r,
            domain: FREQUENCY,
            x: Axis::logarithmic(20., sr as f64 / 2.),
            y: [Some(Axis::linear(-90., 0.)), None],
            unit: "dBFS",
        }
    }
    fn history_plot(&self, r: Area, spectrogram: bool, sr: f32) -> Plot {
        let (start, end) = if spectrogram {
            let end = self.times.back().copied().unwrap_or(0.);
            (self.times.front().copied().unwrap_or(end), end)
        } else {
            (
                self.history.front().map(|p| p.time).unwrap_or(0.),
                self.history
                    .back()
                    .map(|p| p.time)
                    .unwrap_or(0.05)
                    .max(self.history.front().map(|p| p.time).unwrap_or(0.) + 0.05),
            )
        };
        Plot {
            area: r,
            domain: TIME,
            x: Axis::linear(start, end.max(start + 0.00001)),
            y: [
                Some(if spectrogram {
                    Axis::logarithmic(20., sr as f64 / 2.)
                } else {
                    Axis::logarithmic(55., 1760.)
                }),
                None,
            ],
            unit: "Hz",
        }
    }
    fn wave(
        &self,
        c: &CanvasRenderingContext2d,
        r: Area,
        s: &[f32; SNAPSHOT_LEN],
        sr: f32,
        playing: bool,
        frozen: bool,
    ) -> bool {
        let count = (self.settings.time * 10. * sr as f64 / 1000.)
            .round()
            .clamp(2., 1024.) as usize;
        let edge = wave_edge(s, &self.settings, sr);
        let triggered = edge.is_some();
        text(
            "trigger-led",
            if triggered {
                "TRIGGERED"
            } else if self.settings.trigger_mode == 0 {
                "AUTO"
            } else {
                "WAITING"
            },
        );
        let _ = el("trigger-led")
            .class_list()
            .toggle_with_force("triggered", triggered);
        // Normal and single acquisitions must leave the last trace untouched.
        if edge.is_none() && self.settings.trigger_mode != 0 && !frozen {
            return false;
        }
        let start = edge.unwrap_or(WAVE - count);
        c.save();
        c.begin_path();
        c.rect(r.x, r.y, r.w, r.h);
        c.clip();
        inspection::begin(c, self.wave_plot(r, s, sr, None));
        for channel in 0..2 {
            if !self.settings.shown[channel] {
                continue;
            }
            let data = &s[channel * WAVE + start..channel * WAVE + start + count];
            let ac = if self.settings.coupling[channel] == 0 {
                s[channel * WAVE..(channel + 1) * WAVE].iter().sum::<f32>() / WAVE as f32
            } else {
                0.
            };
            let zero = r.y + r.h / 2. - self.settings.position[channel] * r.h / 8.;
            let sensitivity = self.settings.sensitivity[channel];
            let ground = self.settings.coupling[channel] == 2;
            let bins = decimate_extrema(data, r.w as usize);
            c.set_stroke_style_str(if channel == 0 { "#72ffe0" } else { "#a6ffb2" });
            c.set_line_width(1.5);
            c.begin_path();
            for (i, (lo, hi)) in bins.iter().enumerate() {
                let x = r.x + r.w * i as f64 / bins.len().max(1) as f64;
                let a = if ground { 0. } else { *lo - ac };
                let b = if ground { 0. } else { *hi - ac };
                let y1 = zero - a as f64 * r.h / (8. * sensitivity);
                let y2 = zero - b as f64 * r.h / (8. * sensitivity);
                if i == 0 {
                    c.move_to(x, y1);
                } else {
                    c.line_to(x, y1);
                }
                if (y2 - y1).abs() > 0.6 {
                    c.line_to(x, y2);
                }
            }
            c.stroke();
        }
        inspection::end(c);
        c.restore();
        self.settings.trigger_mode == 2 && triggered && playing && !frozen && self.fresh
    }
    fn spectrum(
        &self,
        c: &CanvasRenderingContext2d,
        r: Area,
        s: &[f32; SNAPSHOT_LEN],
        sr: f32,
        envelopes: bool,
    ) {
        inspection::begin(c, Self::frequency_plot(r, sr));
        for (ch, offset) in [FFT_A, FFT_B].into_iter().enumerate() {
            if self.settings.shown[ch] {
                spectrum_line(c, r, &s[offset..offset + 512], sr, ch);
            }
        }
        inspection::end(c);
        if envelopes {
            self.envelopes(c, r, s, sr);
        }
        c.set_shadow_blur(0.);
        inspection::begin(c, Self::frequency_plot(r, sr));
        let pitch = s[META];
        if pitch > 20. {
            c.set_stroke_style_str("#8cbf9a");
            for harmonic in 1..=6 {
                let hz = pitch * harmonic as f32;
                if hz < sr / 2. {
                    let x = r.x + (hz as f64 / 20.).ln() / ((sr as f64 / 2.) / 20.).ln() * r.w;
                    line(c, x, r.y + r.h - 14., x, r.y + r.h);
                }
            }
        }
        inspection::end(c);
    }
    fn envelopes(&self, c: &CanvasRenderingContext2d, r: Area, s: &[f32; SNAPSHOT_LEN], sr: f32) {
        inspection::begin(c, Self::frequency_plot(r, sr));
        for (i, offset) in [ENV_A, ENV_IDEAL, ENV_B].into_iter().enumerate() {
            if self.settings.envelopes[i] {
                c.set_shadow_blur(0.);
                c.set_stroke_style_str(["#8fc9a5", "#b5c6d9", "#d7d49d"][i]);
                let _ = c.set_line_dash(&if i == 1 {
                    Array::of2(&4.into(), &4.into()).into()
                } else {
                    Array::new().into()
                });
                c.begin_path();
                let mut first = true;
                for bin in 1..512 {
                    let hz = bin as f64 * sr as f64 / 1024.;
                    if hz < 20. {
                        continue;
                    }
                    let x = r.x + (hz / 20.).ln() / ((sr as f64 / 2.) / 20.).ln() * r.w;
                    let y =
                        r.y + r.h - ((db(s[offset + bin] / 256.) + 90.) / 90.).clamp(0., 1.) * r.h;
                    if first {
                        c.move_to(x, y);
                        first = false;
                    } else {
                        c.line_to(x, y);
                    }
                }
                c.stroke();
            }
        }
        let _ = c.set_line_dash(&Array::new());
        inspection::end(c);
        label(c, "Original / Ideal / Measured", r.x, r.y - 12.);
    }
    fn spectrograms(&self, c: &CanvasRenderingContext2d, r: Area, sr: f32) {
        for ch in 0..2 {
            let area = Area {
                x: r.x,
                y: r.y + ch as f64 * (r.h / 2. + 17.),
                w: r.w,
                h: r.h / 2. - 22.,
            };
            if let Some(w) = &self.waterfalls {
                let view = inspection::raster(self.history_plot(area, true, sr));
                // Crop from the stored history, never from a resampled screen.
                let _ = c
                    .draw_image_with_html_canvas_element_and_sw_and_sh_and_dx_and_dy_and_dw_and_dh(
                        &w[ch],
                        512. - self.times.len() as f64
                            + view.start() * self.times.len().saturating_sub(1) as f64,
                        0.,
                        self.times.len().saturating_sub(1).max(1) as f64 / view.zoom,
                        128.,
                        area.x,
                        area.y,
                        area.w,
                        area.h,
                    );
            }
            label(
                c,
                if ch == 0 { "INPUT" } else { "OUTPUT" },
                area.x + 8.,
                area.y + 20.,
            );
        }
    }
    fn pitch(&self, c: &CanvasRenderingContext2d, r: Area) {
        let end = self.history.back().map(|p| p.time).unwrap_or(0.);
        let start = self.history.front().map(|p| p.time).unwrap_or(end - 6.);
        let span = (end - start).max(0.05);
        for hz in [55., 110., 220., 440., 880., 1760.] {
            let y = r.y + r.h - (hz / 55f64).log2() / 5. * r.h;
            c.set_stroke_style_str("#44695a");
            line(c, r.x, y, r.x + r.w, y);
            label(
                c,
                &format!("{} / {hz:.0} Hz", note_name(hz as f32)),
                r.x + 4.,
                y - 6.,
            );
        }
        inspection::begin(c, self.history_plot(r, false, 48000.));
        for ch in 0..2 {
            if !self.settings.shown[ch] {
                continue;
            }
            let mut run = Vec::new();
            for p in &self.history {
                let hz = if ch == 0 { p.a } else { p.b };
                if hz >= 50. {
                    run.push((
                        r.x + (p.time - start) / span * r.w,
                        r.y + r.h - ((hz as f64 / 55.).log2() / 5.).clamp(0., 1.) * r.h,
                    ));
                } else if !run.is_empty() {
                    stroke(c, run.drain(..), ch);
                }
            }
            stroke(c, run.into_iter(), ch);
        }
        let _ = c.set_line_dash(&Array::of2(&5.into(), &5.into()));
        c.set_stroke_style_str("#c9d9a5");
        c.begin_path();
        let mut first = true;
        for p in &self.history {
            if p.target > 50. {
                let x = r.x + (p.time - start) / span * r.w;
                let y = r.y + r.h - ((p.target as f64 / 55.).log2() / 5.).clamp(0., 1.) * r.h;
                if first {
                    c.move_to(x, y);
                    first = false;
                } else {
                    c.line_to(x, y);
                }
            }
        }
        c.stroke();
        let _ = c.set_line_dash(&Array::new());
        inspection::end(c);
    }
    fn compare(&self, c: &CanvasRenderingContext2d, r: Area, s: &[f32; SNAPSHOT_LEN], sr: f32) {
        let col = (r.w - 28.) / 2.;
        let height = (r.h - 90.) / 4.;
        for ch in 0..2 {
            let x = r.x + ch as f64 * (col + 28.);
            label(c, if ch == 0 { "INPUT" } else { "OUTPUT" }, x, r.y - 11.);
            for row in 0..4 {
                let a = Area {
                    x,
                    y: r.y + row as f64 * (height + 30.),
                    w: col,
                    h: height,
                };
                lattice(c, a, 5, 2);
                if !self.settings.shown[ch] {
                    continue;
                }
                if row != 2 {
                    let plot = match row {
                        0 => self.wave_plot(a, s, sr, Some(ch)),
                        1 => Self::frequency_plot(a, sr),
                        _ => self.history_plot(a, false, sr),
                    };
                    inspection::begin(c, plot);
                }
                match row {
                    0 => {
                        let count = (self.settings.time * 10. * sr as f64 / 1000.)
                            .round()
                            .clamp(2., 1024.) as usize;
                        let start = wave_edge(s, &self.settings, sr).unwrap_or(WAVE - count);
                        let data = &s[ch * WAVE + start..ch * WAVE + start + count];
                        let mean = if self.settings.coupling[ch] == 0 {
                            s[ch * WAVE..(ch + 1) * WAVE].iter().sum::<f32>() / WAVE as f32
                        } else {
                            0.
                        };
                        let zero = a.y + a.h / 2. - self.settings.position[ch] * a.h / 8.;
                        c.save();
                        c.begin_path();
                        c.rect(a.x, a.y, a.w, a.h);
                        c.clip();
                        stroke(
                            c,
                            data.iter().enumerate().map(|(i, v)| {
                                (
                                    a.x + i as f64 / (count - 1) as f64 * a.w,
                                    zero - if self.settings.coupling[ch] == 2 {
                                        0.
                                    } else {
                                        (*v - mean) as f64 * a.h
                                            / (8. * self.settings.sensitivity[ch])
                                    },
                                )
                            }),
                            ch,
                        );
                        c.restore();
                    }
                    1 => spectrum_line(
                        c,
                        a,
                        &s[if ch == 0 { FFT_A } else { FFT_B }..if ch == 0 {
                            FFT_A + 512
                        } else {
                            FFT_B + 512
                        }],
                        sr,
                        ch,
                    ),
                    2 => {
                        if let Some(w) = &self.waterfalls {
                            let view = inspection::raster(self.history_plot(a, true, sr));
                            let _=c.draw_image_with_html_canvas_element_and_sw_and_sh_and_dx_and_dy_and_dw_and_dh(&w[ch],512.-self.times.len() as f64+view.start()*self.times.len().saturating_sub(1) as f64,0.,self.times.len().saturating_sub(1).max(1) as f64/view.zoom,128.,a.x,a.y,a.w,a.h);
                        }
                    }
                    _ => {
                        let begin = self.history.front().map(|p| p.time).unwrap_or(0.);
                        let end = self.history.back().map(|p| p.time).unwrap_or(begin + 1.);
                        let span = (end - begin).max(0.05);
                        let mut run = Vec::new();
                        for p in &self.history {
                            let hz = if ch == 0 { p.a } else { p.b };
                            if hz > 50. {
                                run.push((
                                    a.x + (p.time - begin) / span * a.w,
                                    a.y + a.h - ((hz as f64 / 55.).log2() / 5.).clamp(0., 1.) * a.h,
                                ));
                            } else {
                                stroke(c, run.drain(..), ch);
                            }
                        }
                        stroke(c, run.into_iter(), ch);
                        let _ = c.set_line_dash(&Array::of2(&4.into(), &4.into()));
                        c.set_stroke_style_str("#d7d49d");
                        c.begin_path();
                        let mut first = true;
                        for p in &self.history {
                            if p.target >= 50. {
                                let x = a.x + (p.time - begin) / span * a.w;
                                let y = a.y + a.h
                                    - ((p.target as f64 / 55.).log2() / 5.).clamp(0., 1.) * a.h;
                                if first {
                                    c.move_to(x, y);
                                    first = false;
                                } else {
                                    c.line_to(x, y);
                                }
                            } else {
                                first = true;
                            }
                        }
                        c.stroke();
                        let _ = c.set_line_dash(&Array::new());
                    }
                }
                if row != 2 {
                    inspection::end(c);
                }
                label(
                    c,
                    [
                        "Waveform · FS",
                        "Spectrum · Hz / dBFS",
                        "Spectrogram · log Hz",
                        "Pitch · Hz",
                    ][row],
                    a.x + 4.,
                    a.y + 16.,
                );
            }
        }
    }
    fn cepstrum(&self, c: &CanvasRenderingContext2d, r: Area, s: &[f32; SNAPSHOT_LEN], sr: f32) {
        let samples = (sr * 0.020).min(511.) as usize;
        let max = s[CEP_A + 1..CEP_A + samples]
            .iter()
            .chain(s[CEP_B + 1..CEP_B + samples].iter())
            .map(|x| x.abs())
            .fold(0.001, f32::max) as f64;
        inspection::begin(
            c,
            Plot {
                area: r,
                domain: PERIOD,
                x: Axis::linear(0., samples as f64 / sr as f64),
                y: [
                    Some(Axis::linear(-0.35 / 0.55 * max, 0.65 / 0.55 * max)),
                    None,
                ],
                unit: "cep",
            },
        );
        for (ch, offset) in [CEP_A, CEP_B].into_iter().enumerate() {
            if self.settings.shown[ch] {
                stroke(
                    c,
                    (1..samples).map(|i| {
                        (
                            r.x + i as f64 / samples as f64 * r.w,
                            r.y + r.h * 0.65 - s[offset + i] as f64 / max * r.h * 0.55,
                        )
                    }),
                    ch,
                );
            }
        }
        inspection::end(c);
        label(c, "Real cepstrum · quefrency", r.x + 5., r.y + 22.);
    }
    fn xy(&self, c: &CanvasRenderingContext2d, r: Area, s: &[f32; SNAPSHOT_LEN]) {
        if !self.settings.shown.iter().all(|shown| *shown) {
            return;
        }
        let means: [f64; 2] = std::array::from_fn(|ch| {
            if self.settings.coupling[ch] == 0 {
                s[ch * WAVE..(ch + 1) * WAVE].iter().sum::<f32>() as f64 / WAVE as f64
            } else {
                0.
            }
        });
        c.save();
        c.begin_path();
        c.rect(r.x, r.y, r.w, r.h);
        c.clip();
        inspection::begin(
            c,
            Plot {
                area: r,
                domain: INPUT,
                x: Axis::linear(
                    -5. * self.settings.sensitivity[0],
                    5. * self.settings.sensitivity[0],
                ),
                y: [
                    Some(Axis::linear(
                        (-4. + self.settings.position[1]) * self.settings.sensitivity[1],
                        (4. + self.settings.position[1]) * self.settings.sensitivity[1],
                    )),
                    None,
                ],
                unit: "FS",
            },
        );
        stroke(
            c,
            (0..WAVE).step_by(2).map(|i| {
                (
                    r.x + r.w / 2.
                        + (if self.settings.coupling[0] == 2 {
                            0.
                        } else {
                            s[i] as f64 - means[0]
                        }) * r.w
                            / (10. * self.settings.sensitivity[0]),
                    r.y + r.h / 2.
                        - self.settings.position[1] * r.h / 8.
                        - (if self.settings.coupling[1] == 2 {
                            0.
                        } else {
                            s[WAVE + i] as f64 - means[1]
                        }) * r.h
                            / (8. * self.settings.sensitivity[1]),
                )
            }),
            0,
        );
        inspection::end(c);
        c.restore();
        label(c, "X: input · Y: output", r.x + 5., r.y + 22.);
    }
}
fn spectrum_line(c: &CanvasRenderingContext2d, r: Area, m: &[f32], sr: f32, ch: usize) {
    c.save();
    c.begin_path();
    c.rect(r.x, r.y, r.w, r.h);
    c.clip();
    stroke(
        c,
        (1..512).map(|i| {
            let hz = i as f64 * sr as f64 / 1024.;
            (
                r.x + (hz / 20.).ln() / ((sr as f64 / 2.) / 20.).ln() * r.w,
                r.y + r.h - ((db(m[i] / 256.) + 90.) / 90.).clamp(0., 1.) * r.h,
            )
        }),
        ch,
    );
    c.restore();
}
fn draw_midi(
    c: &CanvasRenderingContext2d,
    r: Area,
    midi: Option<&crate::midi::Timeline>,
    position: f64,
    target: f32,
) {
    let Some(m) = midi else {
        label(c, "LOAD MIDI TO VIEW THE SCORE", r.x + 130., r.y + r.h / 2.);
        return;
    };
    let time = m.wrapped_seconds(position);
    let start = (time / 8.).floor() * 8.;
    let x0 = r.x + 30.;
    let width = r.w - 42.;
    let treble = r.y + r.h * 0.23;
    let bass = r.y + r.h * 0.64;
    let gap = (r.h / 35.).clamp(8., 18.);
    c.set_stroke_style_str("#6f9881");
    for top in [treble, bass] {
        for i in 0..5 {
            line(c, x0, top + i as f64 * gap, r.x + r.w, top + i as f64 * gap);
        }
    }
    label(c, "G", r.x + 3., treble + 40.);
    label(c, "F", r.x + 3., bass + 40.);
    inspection::begin(
        c,
        Plot {
            area: Area {
                x: x0,
                y: r.y + 35.,
                w: width,
                h: r.h - 60.,
            },
            domain: TIME,
            x: Axis::linear(position - time + start, position - time + start + 8.),
            y: [None, None],
            unit: "",
        },
    );
    let to = m.notes.partition_point(|n| n.start < start + 8.);
    for n in m.notes[..to].iter().filter(|n| n.end >= start).take(2000) {
        let step = midi_staff_step(n.key);
        let (top, reference) = if n.key >= 60 {
            (treble, 45)
        } else {
            (bass, 33)
        };
        let y = top + (reference - step) as f64 * gap / 2.;
        if y < r.y + 35. || y > r.y + r.h - 36. {
            continue;
        }
        let x = x0 + (n.start - start).max(0.) / 8. * width;
        let active = n.start <= time && n.end > time;
        c.set_fill_style_str(if active { "#b7ffbd" } else { "#81b69f" });
        c.begin_path();
        let _ = c.ellipse(x, y, 7., 5., -0.3, 0., std::f64::consts::TAU);
        c.fill();
        c.set_stroke_style_str(if active { "#b7ffbd" } else { "#81b69f" });
        // The horizontal tail encodes the note's actual duration, including ties
        // crossing this eight-second viewport.
        line(c, x, y, x0 + (n.end - start).clamp(0., 8.) / 8. * width, y);
        let direction = if n.track % 2 == 0 { -1. } else { 1. };
        line(
            c,
            x + direction * -6.,
            y,
            x + direction * -6.,
            y + direction * 32.,
        );
        if y > top + 4. * gap {
            let mut ledger = top + 5. * gap;
            while ledger <= y + 1. {
                line(c, x - 12., ledger, x + 12., ledger);
                ledger += gap;
            }
        } else if y < top {
            let mut ledger = top - gap;
            while ledger >= y - 1. {
                line(c, x - 12., ledger, x + 12., ledger);
                ledger -= gap;
            }
        }
        if matches!(n.key % 12, 1 | 3 | 6 | 8 | 10) {
            label(c, "♯", x - 19., y + 4.);
        }
        if n.channel == 9 {
            label(c, "P", x - 3., y + 23.);
        }
    }
    let playhead = x0 + (time - start) / 8. * width;
    c.set_stroke_style_str("#dbefae");
    line(c, playhead, r.y + 35., playhead, r.y + r.h - 25.);
    inspection::end(c);
    label(
        c,
        &format!(
            "{}/{} · {} tracks · MIDI target {}",
            m.time_signature.0,
            m.time_signature.1,
            m.tracks.len(),
            note_name(target)
        ),
        r.x + 5.,
        r.y + 18.,
    );
}
fn compact(v: f64) -> String {
    format!("{v:.4}")
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}
fn wave_edge(s: &[f32; SNAPSHOT_LEN], settings: &Settings, sr: f32) -> Option<usize> {
    let count = (settings.time * 10. * sr as f64 / 1000.)
        .round()
        .clamp(2., 1024.) as usize;
    let source = settings.trigger_source;
    if settings.coupling[source] == 2 {
        return None;
    }
    let data = &s[source * WAVE..(source + 1) * WAVE];
    let mean = if settings.coupling[source] == 0 {
        data.iter().sum::<f32>() / WAVE as f32
    } else {
        0.
    };
    trigger_index(data, count, settings.level + mean, settings.rising)
}
