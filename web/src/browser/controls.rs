use super::*;
use std::{cell::RefCell, rc::Rc};
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
pub(super) fn mount() {
    let switches = doc().query_selector_all(".coupling").unwrap();
    for n in 0..switches.length() {
        let group: Element = switches.item(n).unwrap().unchecked_into();
        for event in ["pointerdown", "pointermove"] {
            let switch = group.clone();
            bind(group.as_ref(), event, move |e| {
                let p: PointerEvent = e.unchecked_into();
                if p.type_() == "pointerdown" {
                    let _ = switch.set_pointer_capture(p.pointer_id());
                } else if !switch.has_pointer_capture(p.pointer_id()) {
                    return;
                }
                p.prevent_default();
                let bounds = switch.get_bounding_client_rect();
                let hit = (((p.client_x() as f64 - bounds.left()) / bounds.width()) * 3.)
                    .floor()
                    .clamp(0., 2.) as u8;
                let at = if p.type_() == "pointerdown" {
                    p.target()
                        .and_then(|t| t.dyn_into::<HtmlInputElement>().ok())
                        .and_then(|i| i.value().parse::<u8>().ok())
                        .unwrap_or(hit)
                } else {
                    hit
                };
                let selected: HtmlInputElement = switch
                    .query_selector(&format!("input[value='{at}']"))
                    .unwrap()
                    .unwrap()
                    .unchecked_into();
                let _ = selected.focus();
                if !selected.checked() {
                    selected.set_checked(true);
                    let _ = selected.dispatch_event(&Event::new("change").unwrap());
                }
            });
        }
    }
    let inputs = doc().query_selector_all(".knob-input").unwrap();
    for index in 0..inputs.length() {
        let i: HtmlInputElement = inputs.item(index).unwrap().unchecked_into();
        i.set_disabled(false);
        sync(&i.id());
        bind_dial(&i.id());
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
    let label = crate::control_markup::value_label(id, v);
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
        "aux-view-0" | "aux-view-1" | "aux-view-2" => draw(),
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
