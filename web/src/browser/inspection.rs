//! Screen interaction and calibrated overlays. No extra audio/FFT work is requested.
use super::scope::Area;
use super::*;
use crate::inspection_math::{Axis, View, period_hz};

pub(super) const TIME: usize = 0;
pub(super) const FREQUENCY: usize = 1;
pub(super) const PERIOD: usize = 2;
pub(super) const INPUT: usize = 3;
#[derive(Clone, Copy)]
pub(super) struct Plot {
    pub area: Area,
    pub domain: usize,
    pub x: Axis,
    pub y: [Option<Axis>; 2],
    pub unit: &'static str,
}
#[derive(Clone, Copy)]
struct Point {
    screen: usize,
    plot: usize,
    domain: usize,
    x: f64,
    y: f64,
    levels: [Option<f64>; 2],
    unit: &'static str,
}
#[derive(Default)]
struct State {
    screen: usize,
    plots: [Vec<Plot>; 4],
    views: [[View; 4]; 4],
    active: usize,
    plot: usize,
    enabled: bool,
    selected: usize,
    cursors: [Option<Point>; 2],
    hover: Option<Point>,
    anchor: Option<Point>,
    dragging: Option<i32>,
    expanded: Option<(Element, Element)>,
}
thread_local! { static STATE: RefCell<State> = RefCell::new(State::default()); }
thread_local! { static DRAW_PENDING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
fn queue_draw() {
    if DRAW_PENDING.replace(true) {
        return;
    }
    let callback = Closure::once_into_js(|| {
        DRAW_PENDING.set(false);
        draw();
    });
    let _ = window()
        .unwrap()
        .request_animation_frame(callback.unchecked_ref());
}
pub(super) fn reset_capture() {
    STATE.with_borrow_mut(|s| {
        s.cursors = [None, None];
        s.hover = None;
        s.anchor = None;
        s.views = [[View::default(); 4]; 4];
    });
}
fn id(screen: usize) -> String {
    if screen == 0 {
        "scope".into()
    } else {
        format!("aux-{}", screen - 1)
    }
}
fn name(screen: usize) -> String {
    if screen == 0 {
        "MAIN DISPLAY".into()
    } else {
        format!("DISPLAY {screen}")
    }
}
fn canvas(screen: usize) -> HtmlCanvasElement {
    el(&id(screen)).unchecked_into()
}
pub(super) fn start(screen: usize) {
    STATE.with_borrow_mut(|s| {
        s.screen = screen;
        s.plots[screen].clear();
    });
}
fn register(s: &mut State, plot: Plot) {
    if !s.plots[s.screen]
        .iter()
        .any(|p| p.area.x == plot.area.x && p.area.y == plot.area.y && p.domain == plot.domain)
    {
        s.plots[s.screen].push(plot);
    }
}
pub(super) fn begin(c: &CanvasRenderingContext2d, plot: Plot) {
    let view = STATE.with_borrow_mut(|s| {
        register(s, plot);
        s.views[s.screen][plot.domain]
    });
    let r = plot.area;
    c.save();
    c.begin_path();
    c.rect(r.x, r.y, r.w, r.h);
    c.clip();
    let _ = c.translate(r.x, r.y);
    let _ = c.scale(view.zoom, 1.);
    let _ = c.translate(-r.x - view.start() * r.w, -r.y);
}
pub(super) fn end(c: &CanvasRenderingContext2d) {
    c.restore();
}
// Register a plot whose raster image is cropped directly instead of transformed.
pub(super) fn raster(plot: Plot) -> View {
    STATE.with_borrow_mut(|s| {
        register(s, plot);
        s.views[s.screen][plot.domain]
    })
}
fn label(c: &CanvasRenderingContext2d, text: &str, x: f64, y: f64) {
    c.set_fill_style_str("#b3e3cf");
    let _ = c.fill_text(text, x, y);
}
fn fmt(v: f64, unit: &str) -> String {
    if unit == "s" && v.abs() < 1. {
        format!("{:.2} ms", v * 1000.)
    } else if unit == "Hz" && v.abs() >= 1000. {
        format!("{:.2} kHz", v / 1000.)
    } else {
        format!("{v:.3} {unit}")
    }
}
fn x_unit(domain: usize) -> &'static str {
    ["s", "Hz", "s", "FS"][domain]
}
fn point_text(p: Point, s: &State) -> String {
    let Some(_plot) = s.plots[p.screen].get(p.plot) else {
        return "—".into();
    };
    let values = p
        .levels
        .iter()
        .enumerate()
        .filter_map(|(ch, level)| {
            level.map(|v| {
                format!(
                    "{}{}",
                    if p.levels[1].is_some() {
                        if ch == 0 { "CH1 " } else { "CH2 " }
                    } else {
                        ""
                    },
                    fmt(v, p.unit)
                )
            })
        })
        .collect::<Vec<_>>()
        .join(" / ");
    format!(
        "{}{}{}",
        fmt(p.x, x_unit(p.domain)),
        if values.is_empty() { "" } else { " · " },
        values
    )
}
fn readout(s: &State) -> String {
    let mut parts = vec![name(s.active)];
    if !s.enabled {
        parts.push("Tap a screen to enlarge. CURSORS to measure.".into());
    } else {
        for (i, p) in s.cursors.iter().enumerate() {
            parts.push(format!(
                "{} {}",
                if i == 0 { "A" } else { "B" },
                p.map(|p| point_text(p, s)).unwrap_or_else(|| "—".into())
            ));
        }
        if let [Some(a), Some(b)] = s.cursors {
            if a.domain == b.domain {
                let delta = b.x - a.x;
                parts.push(format!("Δ {}", fmt(delta, x_unit(a.domain))));
                if matches!(a.domain, TIME | PERIOD) {
                    if let Some(hz) = period_hz(delta) {
                        parts.push(format!("1/|Δ| {}", fmt(hz, "Hz")));
                    }
                }
                if a.screen == b.screen && a.plot == b.plot && a.unit == b.unit {
                    if let (Some(ay), Some(by)) = (a.levels[0], b.levels[0]) {
                        parts.push(format!("ΔY {}", fmt(by - ay, a.unit)));
                    }
                }
            } else {
                parts.push("A/B axes differ".into());
            }
        }
        if let Some(p) = s.hover {
            parts.push(format!("Pointer {}", point_text(p, s)));
        }
    }
    let domain = s.plots[s.active]
        .get(s.plot)
        .map(|p| p.domain)
        .unwrap_or(TIME);
    parts.push(format!("{:.0}×", s.views[s.active][domain].zoom));
    parts.join("  ·  ")
}
pub(super) fn finish(c: &CanvasRenderingContext2d) {
    STATE.with_borrow(|s| {
        let screen = s.screen;
        c.save();
        c.set_shadow_blur(0.);
        c.set_font(if screen == 0 {
            "18px monospace"
        } else {
            "20px monospace"
        });
        for (index, p) in s.plots[screen].iter().enumerate() {
            let r = p.area;
            let view = s.views[screen][p.domain];
            let axis = p.x.slice(view);
            // Compact endpoints stay inside each plot, including COMPARE's eight panels.
            label(
                c,
                &fmt(axis.lo, x_unit(p.domain)),
                r.x + 3.,
                r.y + r.h + 20.,
            );
            c.set_text_align("right");
            label(
                c,
                &fmt(axis.hi, x_unit(p.domain)),
                r.x + r.w - 3.,
                r.y + r.h + 20.,
            );
            c.set_text_align("left");
            if let Some(y) = p.y[0] {
                c.set_font(if screen == 0 {
                    "16px monospace"
                } else {
                    "18px monospace"
                });
                c.set_text_align("right");
                label(c, &fmt(y.hi, p.unit), r.x + r.w - 3., r.y + 44.);
                c.set_text_align("left");
                c.set_font(if screen == 0 {
                    "18px monospace"
                } else {
                    "20px monospace"
                });
            }
            if !s.enabled {
                continue;
            }
            c.save();
            c.begin_path();
            c.rect(r.x, r.y, r.w, r.h);
            c.clip();
            for (i, point) in s.cursors.iter().enumerate() {
                let Some(point) = point.filter(|pt| pt.domain == p.domain) else {
                    continue;
                };
                let fraction = axis.fraction(point.x);
                if !fraction.is_finite() || !(0. ..=1.).contains(&fraction) {
                    continue;
                }
                let x = r.x + fraction * r.w;
                c.set_stroke_style_str(if i == 0 { "#ffe39a" } else { "#e0b9ff" });
                c.set_line_width(1.7);
                c.begin_path();
                c.move_to(x, r.y);
                c.line_to(x, r.y + r.h);
                c.stroke();
                c.set_fill_style_str(if i == 0 { "#ffe39a" } else { "#e0b9ff" });
                let _ = c.fill_text(if i == 0 { "A" } else { "B" }, x + 4., r.y + 38.);
                if point.screen == screen && point.plot == index && point.unit == p.unit {
                    let y = r.y
                        + point.levels[0]
                            .zip(p.y[0])
                            .map(|(v, a)| 1. - a.fraction(v))
                            .unwrap_or(point.y)
                            * r.h;
                    c.begin_path();
                    c.move_to(r.x, y);
                    c.line_to(r.x + r.w, y);
                    c.stroke();
                }
            }
            if let Some(point) = s.hover.filter(|pt| pt.screen == screen && pt.plot == index) {
                let x = r.x + axis.fraction(point.x) * r.w;
                let y = r.y + point.y * r.h;
                c.set_stroke_style_str("#c5edda88");
                c.set_line_width(1.);
                let _ = c.set_line_dash(&Array::of2(&3.into(), &4.into()));
                c.begin_path();
                c.move_to(x, r.y);
                c.line_to(x, r.y + r.h);
                c.move_to(r.x, y);
                c.line_to(r.x + r.w, y);
                c.stroke();
            }
            c.restore();
        }
        c.restore();
        text("inspection-readout", &readout(s));
    });
}
fn hit(screen: usize, e: &PointerEvent) -> Option<Point> {
    let canvas = canvas(screen);
    let r = canvas.get_bounding_client_rect();
    let x = (e.client_x() as f64 - r.x()) / r.width() * canvas.width() as f64;
    let y = (e.client_y() as f64 - r.y()) / r.height() * canvas.height() as f64;
    STATE.with_borrow(|s| {
        s.plots[screen].iter().enumerate().find_map(|(index, p)| {
            let r = p.area;
            if x < r.x || x > r.x + r.w || y < r.y || y > r.y + r.h {
                return None;
            }
            Some(Point {
                screen,
                plot: index,
                domain: p.domain,
                x: p.x.slice(s.views[screen][p.domain]).at((x - r.x) / r.w),
                y: (y - r.y) / r.h,
                levels: p.y.map(|a| a.map(|a| a.at(1. - (y - r.y) / r.h))),
                unit: p.unit,
            })
        })
    })
}
fn cursor_place(point: Point) {
    STATE.with_borrow_mut(|s| {
        s.active = point.screen;
        s.plot = point.plot;
        s.cursors[s.selected] = Some(point);
        s.hover = Some(point);
        s.anchor = Some(point);
    });
}
fn zoom(factor: f64, pan: f64) {
    STATE.with_borrow_mut(|s| {
        let Some(p) = s.plots[s.active].get(s.plot).copied() else {
            return;
        };
        let view = &mut s.views[s.active][p.domain];
        if factor > 1. {
            if let Some(h) = s
                .anchor
                .filter(|h| h.screen == s.active && h.domain == p.domain)
            {
                view.center = p.x.fraction(h.x).clamp(0., 1.);
            }
        }
        view.zoom_by(factor);
        view.pan(pan);
    });
    draw();
}
fn restore() {
    hide_tools();
    let expanded = STATE.with_borrow_mut(|s| s.expanded.take());
    if let Some((hood, placeholder)) = expanded {
        let parent = placeholder.parent_node().unwrap();
        let _ = parent.replace_child(&hood, &placeholder);
        doc()
            .body()
            .unwrap()
            .append_child(&el("inspection-home"))
            .unwrap();
        let dialog: HtmlDialogElement = el("screen-dialog").unchecked_into();
        dialog.close();
        let screen = STATE.with_borrow(|s| s.active);
        let _ = canvas(screen).focus();
        el("screen-enlarge")
            .set_attribute("aria-expanded", "false")
            .unwrap();
    }
}
pub(super) fn sync(frozen: bool) {
    for (i, id) in ["cursor-a", "cursor-b"].into_iter().enumerate() {
        let _ = el(id).set_attribute(
            "aria-pressed",
            if STATE.with_borrow(|s| s.selected == i) {
                "true"
            } else {
                "false"
            },
        );
    }
    let _ = el("screen-dialog")
        .class_list()
        .toggle_with_force("powered", el_instrument().class_list().contains("powered"));
    let _ = doc().document_element().unwrap().set_attribute(
        "data-cursors",
        if STATE.with_borrow(|s| s.enabled) {
            "true"
        } else {
            "false"
        },
    );
    el("screen-hold")
        .set_attribute("aria-pressed", if frozen { "true" } else { "false" })
        .unwrap();
    text("screen-hold", if frozen { "RELEASE" } else { "HOLD" });
    STATE.with_borrow(|s| text("inspection-readout", &readout(s)));
    STATE.with_borrow(|s| {
        for screen in 0..4 {
            let summary = if s.enabled && s.active == screen {
                match s.cursors {
                    [Some(a), Some(b)] if a.domain == b.domain => format!(
                        "A {} · B {} · Δ {}",
                        fmt(a.x, x_unit(a.domain)),
                        fmt(b.x, x_unit(b.domain)),
                        fmt(b.x - a.x, x_unit(a.domain))
                    ),
                    _ => "Tap to place A/B · arrow keys move precisely".into(),
                }
            } else {
                String::new()
            };
            text(&format!("screen-meter-{screen}"), &summary);
        }
    });
}
fn hide_tools() {
    el("inspection-home").set_attribute("hidden", "").unwrap();
    for screen in 0..4 {
        let _ = el(&format!("inspect-screen-{screen}")).set_attribute("aria-expanded", "false");
    }
    let _ = el("inspect-expanded").set_attribute("aria-expanded", "false");
}
fn show_tools(screen: usize) {
    let same = STATE.with_borrow(|s| s.active == screen);
    if same && !el("inspection-home").has_attribute("hidden") {
        hide_tools();
        return;
    }
    STATE.with_borrow_mut(|s| {
        s.active = screen;
        s.plot = s.plot.min(s.plots[screen].len().saturating_sub(1));
    });
    hide_tools();
    text("inspection-title", &format!("{} · INSPECT", name(screen)));
    let home = el("inspection-home");
    home.remove_attribute("hidden").unwrap();
    sync(APP.with_borrow(|a| a.frozen));
    let expanded = STATE.with_borrow(|s| s.expanded.is_some());
    let r = el(&if expanded {
        "inspect-expanded".into()
    } else {
        format!("inspect-screen-{screen}")
    })
    .get_bounding_client_rect();
    let width = window()
        .unwrap()
        .inner_width()
        .unwrap()
        .as_f64()
        .unwrap_or(800.);
    let height = window()
        .unwrap()
        .inner_height()
        .unwrap()
        .as_f64()
        .unwrap_or(600.);
    let bounds = home.get_bounding_client_rect();
    let left = (r.right() - bounds.width()).clamp(8., (width - bounds.width() - 8.).max(8.));
    let top = if r.bottom() + bounds.height() + 8. < height {
        r.bottom() + 6.
    } else {
        (r.top() - bounds.height() - 6.).max(8.)
    };
    let top = top.min((height - bounds.height() - 8.).max(8.));
    home.set_attribute("style", &format!("left:{left}px;top:{top}px"))
        .unwrap();
    el(&format!("inspect-screen-{screen}"))
        .set_attribute("aria-expanded", "true")
        .unwrap();
    if expanded {
        let _ = el("inspect-expanded").set_attribute("aria-expanded", "true");
    }
    el("control-tip").set_attribute("hidden", "").unwrap();
}
fn enlarge() {
    hide_tools();
    if STATE.with_borrow(|s| s.expanded.is_some()) {
        restore();
        return;
    }
    let screen = STATE.with_borrow(|s| s.active);
    let hood = canvas(screen)
        .closest(if screen == 0 {
            ".crt-hood"
        } else {
            ".aux-hood"
        })
        .unwrap()
        .unwrap();
    let placeholder = doc().create_element("div").unwrap();
    let r = hood.get_bounding_client_rect();
    placeholder
        .set_attribute(
            "style",
            &format!(
                "width:{}px;max-width:100%;height:{}px",
                r.width(),
                r.height()
            ),
        )
        .unwrap();
    hood.parent_node()
        .unwrap()
        .replace_child(&placeholder, &hood)
        .unwrap();
    el("screen-socket").append_child(&hood).unwrap();
    el("inspection-expanded-tools")
        .append_child(&el("inspection-home"))
        .unwrap();
    text("screen-dialog-title", &name(screen));
    STATE.with_borrow_mut(|s| s.expanded = Some((hood, placeholder)));
    let dialog: HtmlDialogElement = el("screen-dialog").unchecked_into();
    let _ = dialog.show_modal();
    el("screen-enlarge")
        .set_attribute("aria-expanded", "true")
        .unwrap();
    let _ = canvas(screen).focus();
    draw();
}
fn export() -> Result<(), JsValue> {
    let image: HtmlCanvasElement = doc().create_element("canvas")?.unchecked_into();
    image.set_width(1500);
    image.set_height(1830);
    let c: CanvasRenderingContext2d = image.get_context("2d")?.unwrap().unchecked_into();
    c.set_fill_style_str("#092426");
    c.fill_rect(0., 0., 1500., 1830.);
    c.set_font("24px monospace");
    label(&c, "SynthPhone DSP Viewer · captured screens", 30., 40.);
    for screen in 1..=3 {
        let x = 30. + (screen - 1) as f64 * 490.;
        label(&c, &name(screen), x, 82.);
        c.draw_image_with_html_canvas_element_and_dw_and_dh(&canvas(screen), x, 95., 460., 307.)?;
    }
    c.draw_image_with_html_canvas_element_and_dw_and_dh(
        &el("grid").unchecked_into(),
        125.,
        435.,
        1250.,
        1000.,
    )?;
    c.draw_image_with_html_canvas_element_and_dw_and_dh(&canvas(0), 125., 435., 1250., 1000.)?;
    c.set_font("18px monospace");
    let summary = STATE.with_borrow(readout);
    for (i, chunk) in summary
        .chars()
        .collect::<Vec<_>>()
        .chunks(130)
        .take(3)
        .enumerate()
    {
        label(
            &c,
            &chunk.iter().collect::<String>(),
            25.,
            1455. + i as f64 * 23.,
        );
    }
    let settings = APP.with_borrow(|a| {
        format!(
            "{} · {} · {:.3} s · {} · FFT 1024 · {} Hz · CH1 {:.3} FS/div / CH2 {:.3} FS/div",
            el("view-label").text_content().unwrap_or_default(),
            el("screen-state").text_content().unwrap_or_default(),
            a.snapshot[META + 10],
            el("source-name").text_content().unwrap_or_default(),
            a.snapshot[META + 9],
            a.display.settings.sensitivity[0],
            a.display.settings.sensitivity[1]
        )
    });
    let mut settings = settings;
    settings.push_str(&format!(
        " · Effect {} · Scale {}",
        select("effect").value(),
        input("scale").value()
    ));
    let knobs = doc().query_selector_all("input.knob-input")?;
    for i in 0..knobs.length() {
        let knob: HtmlInputElement = knobs.item(i).unwrap().unchecked_into();
        settings.push_str(&format!(
            " · {}={:.3}",
            knob.id(),
            controls::read(&knob.id())
        ));
    }
    for (i, chunk) in settings.chars().collect::<Vec<_>>().chunks(130).enumerate() {
        label(
            &c,
            &chunk.iter().collect::<String>(),
            25.,
            1540. + i as f64 * 23.,
        );
    }
    let a: HtmlAnchorElement = doc().create_element("a")?.unchecked_into();
    a.set_href(&image.to_data_url_with_type("image/png")?);
    a.set_download("dsp-viewer-capture.png");
    a.set_id("capture-download");
    a.set_text_content(Some("Download PNG"));
    el("capture-status").set_text_content(Some("PNG ready: "));
    el("capture-status").append_child(&a)?;
    a.click();
    Ok(())
}
pub(super) fn mount() {
    let home = doc().create_element("div").unwrap();
    home.set_id("inspection-home");
    home.set_class_name("instrument inspection-popover");
    home.set_attribute("hidden", "").unwrap();
    home.set_attribute("role", "dialog").unwrap();
    home.set_attribute("aria-labelledby", "inspection-title")
        .unwrap();
    home.set_inner_html(r#"<div class="inspection-heading"><h2 id="inspection-title">INSPECT</h2><button id="inspection-close">CLOSE</button></div><div id="screen-tools" class="screen-tools" role="group" aria-label="Screen inspection"><fieldset><legend>MEASURE</legend><div class="inspection-buttons"><button id="screen-cursors" aria-pressed="false">CURSORS</button><button id="cursor-a" aria-pressed="true">CURSOR A</button><button id="cursor-b" aria-pressed="false">CURSOR B</button></div></fieldset><fieldset><legend>VIEW</legend><div class="inspection-buttons"><button id="screen-zoom-in">ZOOM +</button><button id="screen-zoom-out">ZOOM −</button><button id="screen-fit">AUTO FIT</button><button id="screen-left">PAN ←</button><button id="screen-right">PAN →</button><button id="screen-enlarge" aria-expanded="false">ENLARGE</button></div></fieldset><fieldset><legend>CAPTURE</legend><div class="inspection-buttons"><button id="screen-hold" aria-pressed="false">HOLD</button><button id="screen-save">SAVE PNG</button></div></fieldset><output id="inspection-readout" aria-label="Cursor measurements"></output><span id="capture-status" role="status" aria-live="polite"></span></div>"#);
    doc().body().unwrap().append_child(&home).unwrap();
    let dialog = doc().create_element("dialog").unwrap();
    dialog.set_id("screen-dialog");
    dialog.set_class_name("instrument inspection-dialog");
    dialog
        .set_attribute("aria-labelledby", "screen-dialog-title")
        .unwrap();
    dialog.set_inner_html(r#"<div class="screen-dialog-header"><h2 id="screen-dialog-title"></h2><button id="inspect-expanded" aria-expanded="false" aria-controls="inspection-home">INSPECT</button><button id="screen-restore">RESTORE</button></div><div id="screen-socket"></div><div id="inspection-expanded-tools"></div>"#);
    doc().body().unwrap().append_child(&dialog).unwrap();
    bind(el("inspect-expanded").as_ref(), "click", |_| {
        show_tools(STATE.with_borrow(|s| s.active))
    });
    bind(el("inspection-close").as_ref(), "click", |_| hide_tools());
    for (id, key) in [
        ("screen-cursors", "C"),
        ("cursor-a", "A"),
        ("cursor-b", "B"),
        ("screen-zoom-in", "+"),
        ("screen-zoom-out", "-"),
        ("screen-left", "["),
        ("screen-right", "]"),
        ("screen-fit", "F"),
        ("screen-enlarge", "E"),
        ("screen-hold", "H Space"),
        ("screen-save", "S"),
    ] {
        let _ = el(id).set_attribute("aria-keyshortcuts", key);
    }
    bind(doc().as_ref(), "pointerdown", |e| {
        if e.target()
            .and_then(|t| t.dyn_into::<Element>().ok())
            .and_then(|e| {
                e.closest("#inspection-home,.inspect-launcher,#inspect-expanded")
                    .ok()
                    .flatten()
            })
            .is_none()
        {
            hide_tools();
        }
    });
    bind(doc().as_ref(), "scroll", |_| hide_tools());
    bind(el("screen-cursors").as_ref(), "click", |_| {
        let on = STATE.with_borrow_mut(|s| {
            s.enabled = !s.enabled;
            s.enabled
        });
        el("screen-cursors")
            .set_attribute("aria-pressed", if on { "true" } else { "false" })
            .unwrap();
        draw();
    });
    for (index, id) in ["cursor-a", "cursor-b"].into_iter().enumerate() {
        bind(el(id).as_ref(), "click", move |_| {
            STATE.with_borrow_mut(|s| {
                s.selected = index;
                s.enabled = true;
            });
            for (i, id) in ["cursor-a", "cursor-b"].into_iter().enumerate() {
                el(id)
                    .set_attribute("aria-pressed", if i == index { "true" } else { "false" })
                    .unwrap();
            }
            el("screen-cursors")
                .set_attribute("aria-pressed", "true")
                .unwrap();
            draw();
        });
    }
    for (id, factor, pan) in [
        ("screen-zoom-in", 2., 0.),
        ("screen-zoom-out", 0.5, 0.),
        ("screen-left", 1., -0.25),
        ("screen-right", 1., 0.25),
    ] {
        bind(el(id).as_ref(), "click", move |_| zoom(factor, pan));
    }
    bind(el("screen-fit").as_ref(), "click", |_| {
        let wave = STATE.with_borrow_mut(|s| {
            s.views[s.active] = [View::default(); 4];
            s.plots[s.active]
                .get(s.plot)
                .is_some_and(|p| p.unit == "FS")
        });
        if wave {
            APP.with_borrow_mut(|a| {
                for ch in 0..2 {
                    let data =
                        &a.snapshot[ch * crate::engine::WAVE..(ch + 1) * crate::engine::WAVE];
                    let mean = if a.display.settings.coupling[ch] == 0 {
                        data.iter().sum::<f32>() / data.len() as f32
                    } else {
                        0.
                    };
                    let peak = data
                        .iter()
                        .map(|v| (v - mean).abs())
                        .fold(0.001f32, f32::max) as f64;
                    let desired = (peak / 1.7).max(0.005);
                    let values = [0.01f64, 0.02, 0.05, 0.1, 0.2, 0.5, 1.];
                    let index = (0..values.len())
                        .min_by(|i, j| {
                            (values[*i] / desired)
                                .ln()
                                .abs()
                                .total_cmp(&(values[*j] / desired).ln().abs())
                        })
                        .unwrap();
                    for (id, value) in [
                        (
                            if ch == 0 { "vertical" } else { "vertical-out" },
                            index as f64,
                        ),
                        (
                            if ch == 0 { "fine-a" } else { "fine-b" },
                            (desired / values[index]).clamp(0.5, 2.),
                        ),
                        (
                            if ch == 0 { "position-a" } else { "position-b" },
                            if ch == 0 { 1.8 } else { -1.8 },
                        ),
                    ] {
                        input(id).set_value_as_number(value);
                        controls::sync(id);
                    }
                }
                a.display.settings.read_controls();
            });
        }
        draw();
    });
    bind(el("screen-enlarge").as_ref(), "click", |_| enlarge());
    bind(el("screen-restore").as_ref(), "click", |_| restore());
    bind(el("screen-dialog").as_ref(), "cancel", |e| {
        e.prevent_default();
        restore();
    });
    bind(el("screen-hold").as_ref(), "click", |_| {
        el("freeze").unchecked_into::<HtmlElement>().click()
    });
    bind(el("screen-save").as_ref(), "click", |_| {
        if let Err(e) = export() {
            error(e)
        }
    });
    bind(doc().as_ref(), "keydown", |event| {
        let e: KeyboardEvent = event.unchecked_into();
        if e.ctrl_key() || e.meta_key() || e.alt_key() || e.repeat() {
            return;
        }
        if let Some(target) = e.target().and_then(|t| t.dyn_into::<Element>().ok()) {
            if target
                .closest("input,select,textarea,[contenteditable=true]")
                .ok()
                .flatten()
                .is_some()
            {
                return;
            }
            if e.key() == " " && target.closest("button").ok().flatten().is_some() {
                return;
            }
        }
        let button = match e.key().to_ascii_lowercase().as_str() {
            "i" => {
                let screen = STATE.with_borrow(|s| s.active);
                show_tools(screen);
                return;
            }
            "escape" if !el("inspection-home").has_attribute("hidden") => {
                e.prevent_default();
                hide_tools();
                let _ = canvas(STATE.with_borrow(|s| s.active)).focus();
                return;
            }
            "c" => "screen-cursors",
            "a" => "cursor-a",
            "b" => "cursor-b",
            "h" | " " => "screen-hold",
            "+" | "=" => "screen-zoom-in",
            "-" | "_" => "screen-zoom-out",
            "[" => "screen-left",
            "]" => "screen-right",
            "f" => "screen-fit",
            "e" => "screen-enlarge",
            "s" => "screen-save",
            "?" => "help-button",
            _ => return,
        };
        // Canvas +/- is handled here once, rather than again in its local key handler.
        e.prevent_default();
        el(button).unchecked_into::<HtmlElement>().click();
    });
    for screen in 0..4 {
        let target = canvas(screen);
        let hood = target
            .closest(if screen == 0 {
                ".crt-hood"
            } else {
                ".aux-hood"
            })
            .unwrap()
            .unwrap();
        let launcher = doc().create_element("button").unwrap();
        launcher.set_id(&format!("inspect-screen-{screen}"));
        launcher.set_class_name("inspect-launcher");
        launcher.set_text_content(Some("INSPECT"));
        launcher
            .set_attribute(
                "aria-label",
                &format!("Inspect {}", name(screen).to_ascii_lowercase()),
            )
            .unwrap();
        launcher.set_attribute("aria-expanded", "false").unwrap();
        launcher
            .set_attribute("aria-controls", "inspection-home")
            .unwrap();
        if screen == 0 {
            let wrapper = doc().create_element("div").unwrap();
            wrapper.set_class_name("main-screen");
            hood.parent_node()
                .unwrap()
                .insert_before(&wrapper, Some(&hood))
                .unwrap();
            let heading = doc().create_element("div").unwrap();
            heading.set_class_name("screen-label");
            heading.set_inner_html("<span>MAIN DISPLAY</span>");
            heading.append_child(&launcher).unwrap();
            wrapper.append_child(&heading).unwrap();
            wrapper.append_child(&hood).unwrap();
        } else {
            let heading = hood
                .parent_element()
                .unwrap()
                .query_selector("h2")
                .unwrap()
                .unwrap();
            heading.append_child(&launcher).unwrap();
        }
        let meter = doc().create_element("output").unwrap();
        meter.set_id(&format!("screen-meter-{screen}"));
        meter.set_class_name("screen-meter");
        hood.append_child(&meter).unwrap();
        bind(launcher.as_ref(), "click", move |_| show_tools(screen));
        target.set_tab_index(0);
        target.set_attribute("aria-description","Tap to enlarge. With cursors on, drag A or B; arrow keys move the selected cursor. Enter enlarges; Escape restores.").unwrap();
        bind(target.as_ref(), "focus", move |_| {
            STATE.with_borrow_mut(|s| {
                s.active = screen;
                s.plot = s.plot.min(s.plots[screen].len().saturating_sub(1));
            });
        });
        bind(target.as_ref(), "pointerdown", move |e| {
            let e: PointerEvent = e.unchecked_into();
            STATE.with_borrow_mut(|s| s.active = screen);
            if !STATE.with_borrow(|s| s.enabled) {
                return;
            }
            if let Some(point) = hit(screen, &e) {
                e.prevent_default();
                let _ = canvas(screen).set_pointer_capture(e.pointer_id());
                let _ = canvas(screen).focus();
                STATE.with_borrow_mut(|s| s.dragging = Some(e.pointer_id()));
                cursor_place(point);
                draw();
            }
        });
        bind(target.as_ref(), "pointermove", move |e| {
            let e: PointerEvent = e.unchecked_into();
            let point = hit(screen, &e);
            let drag = STATE.with_borrow_mut(|s| {
                s.hover = point;
                if let Some(p) = point {
                    s.active = screen;
                    s.plot = p.plot;
                    s.anchor = Some(p);
                }
                s.enabled && s.dragging == Some(e.pointer_id())
            });
            if drag {
                if let Some(p) = point {
                    cursor_place(p);
                }
            }
            if STATE.with_borrow(|s| s.enabled) {
                queue_draw();
            }
        });
        for event in ["pointerup", "pointercancel", "lostpointercapture"] {
            bind(target.as_ref(), event, move |_| {
                STATE.with_borrow_mut(|s| s.dragging = None);
            });
        }
        bind(target.as_ref(), "pointerleave", |_| {
            STATE.with_borrow_mut(|s| s.hover = None);
            draw();
        });
        bind(target.as_ref(), "click", move |_| {
            STATE.with_borrow_mut(|s| s.active = screen);
            if !STATE.with_borrow(|s| s.enabled) {
                enlarge();
            }
        });
        bind(target.as_ref(), "keydown", move |e| {
            let e: KeyboardEvent = e.unchecked_into();
            match e.key().as_str() {
                "Enter" => {
                    e.prevent_default();
                    enlarge();
                }
                "ArrowLeft" | "ArrowRight" | "ArrowUp" | "ArrowDown" => {
                    e.prevent_default();
                    STATE.with_borrow_mut(|s| {
                        s.active = screen;
                        let index = s.plot.min(s.plots[screen].len().saturating_sub(1));
                        let Some(plot) = s.plots[screen].get(index).copied() else {
                            return;
                        };
                        let axis = plot.x.slice(s.views[screen][plot.domain]);
                        let mut p = s.cursors[s.selected]
                            .filter(|p| {
                                p.screen == screen
                                    && p.plot == index
                                    && p.domain == plot.domain
                                    && p.unit == plot.unit
                            })
                            .unwrap_or(Point {
                                screen,
                                plot: index,
                                domain: plot.domain,
                                x: axis.at(0.5),
                                y: 0.5,
                                levels: plot.y.map(|a| a.map(|a| a.at(0.5))),
                                unit: plot.unit,
                            });
                        if let Some((level, axis)) = p.levels[0].zip(plot.y[0]) {
                            p.y = (1. - axis.fraction(level)).clamp(0., 1.);
                        }
                        let step = if e.shift_key() { 0.001 } else { 0.01 };
                        match e.key().as_str() {
                            "ArrowLeft" => p.x = axis.at((axis.fraction(p.x) - step).clamp(0., 1.)),
                            "ArrowRight" => {
                                p.x = axis.at((axis.fraction(p.x) + step).clamp(0., 1.))
                            }
                            "ArrowUp" => p.y = (p.y - step).max(0.),
                            _ => p.y = (p.y + step).min(1.),
                        }
                        s.enabled = true;
                        p.levels = plot.y.map(|a| a.map(|a| a.at(1. - p.y)));
                        s.cursors[s.selected] = Some(p);
                    });
                    el("screen-cursors")
                        .set_attribute("aria-pressed", "true")
                        .unwrap();
                    draw();
                }
                _ => {}
            }
        });
    }
}
