use super::*;
use std::{cell::RefCell, fmt::Write as _, rc::Rc};
pub(super) fn read(id: &str) -> f32 {
    let i = input(id);
    let n = i.value_as_number();
    let n = if n.is_finite() {
        n as f32
    } else {
        i.value().parse().unwrap_or(0.)
    };
    if let Some(v) = i.get_attribute("data-values") {
        v.split(',')
            .nth(n.round().max(0.) as usize)
            .and_then(|v| v.parse().ok())
            .unwrap_or(n)
    } else {
        n
    }
}
fn attribute(e: &Element, key: &str, default: &str) -> String {
    e.get_attribute(key).unwrap_or_else(|| default.into())
}
fn name(id: &str, label: &str) -> String {
    match id {
        "vertical" => "Channel 1 sensitivity".into(),
        "vertical-out" => "Channel 2 sensitivity".into(),
        "position-a" => "Channel 1 vertical position".into(),
        "position-b" => "Channel 2 vertical position".into(),
        "timebase" => "Timebase milliseconds per division".into(),
        _ => label.to_lowercase(),
    }
}
pub(super) fn mount() {
    let mounts = doc().query_selector_all(".knob-mount").unwrap();
    for index in 0..mounts.length() {
        let e: Element = mounts.item(index).unwrap().unchecked_into();
        let id = attribute(&e, "data-id", "");
        let label = attribute(&e, "data-label", &id);
        let values = attribute(&e, "data-values", "");
        let ticks = attribute(&e, "data-ticks", "");
        let minimum = attribute(&e, "data-min", "0");
        let maximum = if values.is_empty() {
            attribute(&e, "data-max", "1")
        } else {
            (values.split(',').count() - 1).to_string()
        };
        let step = attribute(&e, "data-step", "1");
        let default = attribute(&e, "data-default", "0");
        let mut scale =
            String::from("<svg class=\"knob-scale\" viewBox=\"0 0 100 100\" aria-hidden=\"true\">");
        for tick in 0..=27 {
            let a = (-135. + tick as f64 * 10.).to_radians();
            let outer = 38.;
            let inner = if tick % 3 == 0 { 34. } else { 36. };
            write!(
                scale,
                "<line x1=\"{:.2}\" y1=\"{:.2}\" x2=\"{:.2}\" y2=\"{:.2}\"/>",
                50. + inner * a.sin(),
                50. - inner * a.cos(),
                50. + outer * a.sin(),
                50. - outer * a.cos()
            )
            .unwrap();
        }
        let labels = ticks.split(',').collect::<Vec<_>>();
        for (j, label) in labels.iter().enumerate() {
            let a = (-135. + 270. * j as f64 / (labels.len() - 1).max(1) as f64).to_radians();
            write!(
                scale,
                "<text x=\"{:.1}\" y=\"{:.1}\">{label}</text>",
                50. + 46. * a.sin(),
                50. - 46. * a.cos()
            )
            .unwrap();
        }
        scale.push_str("</svg>");
        let html = format!(
            "<label class=\"knob-label\" for=\"{id}\">{label}</label><div class=\"knob-face\">{scale}<span class=\"knob-shaft\"></span><input id=\"{id}\" class=\"knob-input\" type=\"range\" min=\"{minimum}\" max=\"{maximum}\" step=\"{step}\" value=\"{default}\" data-default=\"{default}\" data-values=\"{values}\" aria-label=\"{}\"></div><output class=\"knob-value\" id=\"{id}-value\" for=\"{id}\"></output>",
            name(&id, &label)
        );
        e.set_inner_html(&html);
        sync(&id);
        bind_dial(&id);
        if matches!(id.as_str(), "vertical" | "vertical-out" | "timebase") {
            let fine = match id.as_str() {
                "vertical" => "fine-a",
                "vertical-out" => "fine-b",
                _ => "fine-time",
            };
            let max = if fine == "fine-time" { 1. } else { 2. };
            let nested = doc().create_element("div").unwrap();
            nested.set_class_name("knob-mount fine-knob");
            nested.set_inner_html(&format!("<div class=\"knob-face\"><span class=\"knob-shaft\"></span><span class=\"fine-mark\">VAR</span><input id=\"{fine}\" class=\"knob-input\" type=\"range\" min=\"0.5\" max=\"{max}\" step=\"0.01\" value=\"1\" data-default=\"1\" aria-label=\"{} fine adjustment\"></div><output class=\"sr-only\" id=\"{fine}-value\"></output>", name(&id,&label)));
            e.query_selector(".knob-face")
                .unwrap()
                .unwrap()
                .append_child(&nested)
                .unwrap();
            sync(fine);
            bind_dial(fine);
        }
    }
}
pub(super) fn sync(id: &str) {
    let i = input(id);
    let numeric = i.value_as_number();
    let min = i.min().parse::<f64>().unwrap_or(0.);
    let max = i.max().parse::<f64>().unwrap_or(1.);
    let angle = (numeric - min) / (max - min).max(0.001) * 270. - 135.;
    if let Ok(Some(mount)) = i.closest(".knob-mount") {
        mount
            .set_attribute("style", &format!("--angle:{angle}deg"))
            .unwrap();
    }
    let v = read(id);
    let label = match id {
        "mix" | "strength" | "harmony-strength" => format!("{:.0}%", v * 100.),
        "frequency" | "test-frequency" => format!("{v:.0} Hz"),
        "shift" | "harmony-shift" | "formant" => format!("{v:+.1} st"),
        "bits" => format!("{v:.0} bit"),
        "reduction" => format!("÷{v:.0}"),
        "position-a" | "position-b" => format!("{v:+.2}"),
        "trigger-level" => format!("{v:+.2} FS"),
        "vertical" | "vertical-out" | "timebase" => format!("{v}"),
        _ => format!("{v:.2}"),
    };
    text(&format!("{id}-value"), &label);
    i.set_attribute("aria-valuetext", &label).unwrap();
}
struct Drag {
    origin: f64,
    value: f64,
    cx: f64,
    cy: f64,
    last: f64,
    angle: f64,
    angular: bool,
}
fn bind_dial(id: &str) {
    let id = id.to_string();
    let drag: Rc<RefCell<Option<Drag>>> = Rc::new(RefCell::new(None));
    let down = drag.clone();
    let key = id.clone();
    bind(el(&id).as_ref(), "pointerdown", move |e| {
        let p: PointerEvent = e.unchecked_into();
        p.prevent_default();
        let i = input(&key);
        let r = i.get_bounding_client_rect();
        let (cx, cy) = (r.x() + r.width() / 2., r.y() + r.height() / 2.);
        let (dx, dy) = (p.client_x() as f64 - cx, p.client_y() as f64 - cy);
        let angle = dx.atan2(-dy).to_degrees();
        let _ = i.set_pointer_capture(p.pointer_id());
        let _ = i.focus();
        *down.borrow_mut() = Some(Drag {
            origin: p.client_y() as f64,
            value: i.value_as_number(),
            cx,
            cy,
            last: angle,
            angle: 0.,
            angular: dx.hypot(dy) > r.width() * 0.18,
        });
    });
    let movement = drag.clone();
    let key = id.clone();
    bind(el(&id).as_ref(), "pointermove", move |e| {
        let p: PointerEvent = e.unchecked_into();
        let mut state = movement.borrow_mut();
        let Some(d) = state.as_mut() else { return };
        p.prevent_default();
        let i = input(&key);
        let min = i.min().parse::<f64>().unwrap();
        let max = i.max().parse::<f64>().unwrap();
        let step = i.step().parse::<f64>().unwrap_or(1.);
        let factor = if p.shift_key() { 0.15 } else { 1. };
        let delta = if d.angular {
            let angle = (p.client_x() as f64 - d.cx)
                .atan2(-(p.client_y() as f64 - d.cy))
                .to_degrees();
            let change = (angle - d.last + 180.).rem_euclid(360.) - 180.;
            d.angle += change * factor;
            d.last = angle;
            d.angle / 270.
        } else {
            (d.origin - p.client_y() as f64) * factor / 180.
        };
        let v = ((d.value + delta * (max - min) - min) / step).round() * step + min;
        i.set_value_as_number(v.clamp(min, max));
        sync(&key);
        changed(&key);
    });
    for event in ["pointerup", "pointercancel", "lostpointercapture"] {
        let done = drag.clone();
        bind(el(&id).as_ref(), event, move |_| {
            *done.borrow_mut() = None;
        });
    }
    let key = id.clone();
    bind(el(&id).as_ref(), "input", move |_| {
        sync(&key);
        changed(&key);
    });
    let key = id.clone();
    bind(el(&id).as_ref(), "dblclick", move |_| {
        let i = input(&key);
        i.set_value(&i.get_attribute("data-default").unwrap());
        sync(&key);
        changed(&key);
    });
    let key = id.clone();
    let callback = Closure::<dyn FnMut(Event)>::new(move |e: Event| {
        if doc().active_element().is_none_or(|a| a.id() != key) {
            return;
        }
        let w: WheelEvent = e.unchecked_into();
        w.prevent_default();
        let i = input(&key);
        let step = i.step().parse::<f64>().unwrap_or(1.);
        let min = i.min().parse::<f64>().unwrap();
        let max = i.max().parse::<f64>().unwrap();
        let delta = if w.delta_y() < 0. { step } else { -step };
        i.set_value_as_number((i.value_as_number() + delta).clamp(min, max));
        sync(&key);
        changed(&key);
    });
    let options = AddEventListenerOptions::new();
    options.set_passive(false);
    el(&id)
        .add_event_listener_with_callback_and_add_event_listener_options(
            "wheel",
            callback.as_ref().unchecked_ref(),
            &options,
        )
        .unwrap();
    callback.forget();
}
fn changed(id: &str) {
    match id {
        "vertical" | "vertical-out" | "fine-a" | "fine-b" | "fine-time" | "position-a"
        | "position-b" | "timebase" | "trigger-level" | "intensity" | "focus" | "illumination"
        | "persistence" => {
            APP.with_borrow_mut(|a| a.display.settings.read_controls());
            draw();
        }
        "test-frequency" => retune_test(),
        _ => params(),
    }
}
