use crate::engine::{META, SNAPSHOT_LEN};
use js_sys::{Array, Float32Array, Object, Reflect, Uint8Array};
use std::{cell::RefCell, collections::VecDeque};
use wasm_bindgen::{JsCast, prelude::*};
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::*;
mod controls;
mod scope;
mod tips;
thread_local! {static APP:RefCell<App>=RefCell::new(App::default());}
struct App {
    context: Option<AudioContext>,
    node: Option<AudioWorkletNode>,
    gain: Option<GainNode>,
    source: Option<AudioNode>,
    stream: Option<MediaStream>,
    file: Option<AudioBuffer>,
    midi: Option<Uint8Array>,
    timeline: Option<crate::midi::Timeline>,
    wasm: Option<JsValue>,
    view: u8,
    frozen: bool,
    bypass: bool,
    bypass_until: f64,
    busy: bool,
    pending: bool,
    request: u32,
    pending_view: u8,
    snapshot: [f32; SNAPSHOT_LEN],
    display: scope::Display,
    offset: f64,
    started: f64,
    playing: bool,
    ticks: u32,
    epoch: u64,
    source_revision: u64,
    audio_ticket: u64,
    midi_ticket: u64,
    node_handlers: Vec<(EventTarget, &'static str, Closure<dyn FnMut(Event)>)>,
}
impl Default for App {
    fn default() -> Self {
        Self {
            context: None,
            node: None,
            gain: None,
            source: None,
            stream: None,
            file: None,
            midi: None,
            timeline: None,
            wasm: None,
            view: 6,
            frozen: false,
            bypass: false,
            bypass_until: 0.,
            busy: false,
            pending: false,
            request: 0,
            pending_view: 6,
            snapshot: [0.; SNAPSHOT_LEN],
            display: scope::Display::default(),
            offset: 0.,
            started: 0.,
            playing: false,
            ticks: 0,
            epoch: 0,
            source_revision: 0,
            audio_ticket: 0,
            midi_ticket: 0,
            node_handlers: Vec::new(),
        }
    }
}
fn doc() -> Document {
    window().unwrap().document().unwrap()
}
fn el(id: &str) -> Element {
    doc()
        .get_element_by_id(id)
        .unwrap_or_else(|| panic!("Missing UI element: {id}"))
}
fn input(id: &str) -> HtmlInputElement {
    el(id).unchecked_into()
}
fn value(id: &str) -> f32 {
    controls::read(id)
}
fn select(id: &str) -> HtmlSelectElement {
    el(id).unchecked_into()
}
fn text(id: &str, s: &str) {
    let e = el(id);
    if e.text_content().as_deref() != Some(s) {
        e.set_text_content(Some(s));
    }
}
fn now() -> f64 {
    window().unwrap().performance().unwrap().now()
}
fn get(o: &JsValue, k: &str) -> JsValue {
    Reflect::get(o, &k.into()).unwrap_or(JsValue::UNDEFINED)
}
fn set(o: &JsValue, k: &str, v: impl Into<JsValue>) {
    Reflect::set(o, &k.into(), &v.into()).unwrap();
}
fn message(k: &str, v: impl Into<JsValue>) -> Object {
    let o = Object::new();
    set(&o, k, v);
    o
}
fn bind(t: &EventTarget, event: &str, f: impl FnMut(Event) + 'static) {
    let c = Closure::<dyn FnMut(Event)>::new(f);
    t.add_event_listener_with_callback(event, c.as_ref().unchecked_ref())
        .unwrap();
    c.forget();
}
fn bind_node(t: &EventTarget, event: &'static str, f: impl FnMut(Event) + 'static) {
    let c = Closure::<dyn FnMut(Event)>::new(f);
    t.add_event_listener_with_callback(event, c.as_ref().unchecked_ref())
        .unwrap();
    APP.with_borrow_mut(|a| a.node_handlers.push((t.clone(), event, c)));
}
fn error(e: JsValue) {
    console::error_1(&e);
    text(
        "status",
        &e.as_string()
            .or_else(|| get(&e, "message").as_string())
            .unwrap_or("Audio operation failed. Power off and retry.".into()),
    );
    APP.with_borrow_mut(|a| {
        a.busy = false;
        a.pending = false;
    });
}
fn post(k: &str, v: impl Into<JsValue>) {
    let m = message(k, v);
    APP.with_borrow(|a| {
        if let Some(n) = &a.node {
            let _ = n.port().unwrap().post_message(&m);
        }
    });
}
fn asset(path: &str) -> String {
    let key = match path {
        "engine.wasm" => "data-engine-src",
        "audio-worklet.js" => "data-worklet-src",
        _ => "",
    };
    if let Some(url) = doc().document_element().and_then(|e| e.get_attribute(key)) {
        return url;
    }
    let version = doc()
        .document_element()
        .and_then(|e| e.get_attribute("data-build"))
        .unwrap_or_else(|| "dev".into());
    format!("{path}?v={version}")
}
fn button_bank(attribute: &str, value: &str) {
    let all = doc().query_selector_all(&format!("[{attribute}]")).unwrap();
    for i in 0..all.length() {
        let e: Element = all.item(i).unwrap().unchecked_into();
        let selected = e.get_attribute(attribute).as_deref() == Some(value);
        let _ = e.class_list().toggle_with_force("selected", selected);
        let _ = e.set_attribute("aria-pressed", if selected { "true" } else { "false" });
    }
}
fn params() {
    let mode = select("effect").value().parse::<u8>().unwrap_or(1);
    let bypass = APP.with_borrow(|a| a.bypass || a.bypass_until > now());
    let values = [
        mode as f32,
        if mode == 7 { 1. } else { value("strength") },
        value("shift"),
        value("formant"),
        value("bits"),
        value("reduction"),
        value("frequency"),
        value("mix"),
        if bypass { 1. } else { 0. },
        crate::key::library_key(value("key") as usize, input("scale").value() == "12") as f32,
        value("speed"),
        value("carrier"),
        value("harmony-strength"),
        value("harmony-shift"),
    ];
    post("params", Float32Array::from(values.as_slice()));
    el_instrument()
        .set_attribute("data-mode", &mode.to_string())
        .unwrap();
    button_bank("data-effect", &mode.to_string());
    button_bank("data-carrier", &format!("{:.0}", value("carrier")));
    for (id, enabled) in [
        ("strength", mode == 1),
        ("harmony-strength", mode == 4),
        ("shift", matches!(mode, 1 | 5 | 7)),
        ("harmony-shift", mode == 4),
        ("frequency", matches!(mode, 2 | 6)),
        ("speed", matches!(mode, 1 | 7)),
        ("formant", !matches!(mode, 0 | 3)),
        ("bits", mode != 0),
        ("reduction", mode != 0),
    ] {
        input(id).set_disabled(!enabled);
    }
    el("bypass")
        .set_attribute(
            "aria-pressed",
            if APP.with_borrow(|a| a.bypass) {
                "true"
            } else {
                "false"
            },
        )
        .unwrap();
}
fn el_instrument() -> Element {
    doc().query_selector(".instrument").unwrap().unwrap()
}
fn reset_engine() {
    APP.with_borrow_mut(|a| {
        a.display.reset();
        a.snapshot.fill(0.);
        a.pending = false;
        a.request = a.request.wrapping_add(1);
    });
    post("reset", true);
    params();
    if let Some(m) = APP.with_borrow(|a| a.midi.clone()) {
        post("midi", m);
    }
}
async fn bytes(file: &Blob) -> Result<JsValue, JsValue> {
    JsFuture::from(file.array_buffer()).await
}
async fn start(epoch: u64) -> Result<(), JsValue> {
    let options = AudioContextOptions::new();
    options.set_sample_rate(48000.);
    let c = AudioContext::new_with_context_options(&options)?;
    let resumed = c.resume()?;
    APP.with_borrow_mut(|a| a.context = Some(c.clone()));
    JsFuture::from(resumed).await?;
    let cached = APP.with_borrow(|a| a.wasm.clone());
    let wasm = if let Some(w) = cached {
        w
    } else {
        let response: Response =
            JsFuture::from(window().unwrap().fetch_with_str(&asset("engine.wasm")))
                .await?
                .dyn_into()?;
        if !response.ok() {
            return Err("DSP download failed.".into());
        }
        JsFuture::from(response.array_buffer()?).await?
    };
    if APP.with_borrow(|a| a.epoch != epoch) {
        let _ = c.close();
        return Ok(());
    }
    JsFuture::from(c.audio_worklet()?.add_module(&asset("audio-worklet.js"))?).await?;
    if APP.with_borrow(|a| a.epoch != epoch) {
        let _ = c.close();
        return Ok(());
    }
    let options = AudioWorkletNodeOptions::new();
    options.set_number_of_inputs(1);
    options.set_number_of_outputs(1);
    options.set_output_channel_count(&Array::of1(&1.into()));
    options.set_processor_options(Some(&message("wasm", wasm.clone())));
    let node = AudioWorkletNode::new_with_options(&c, "dsp-bench", &options)?;
    let gain = c.create_gain()?;
    gain.gain()
        .set_value(if input("monitor").checked() { 0.5 } else { 0. });
    node.connect_with_audio_node(&gain)?;
    gain.connect_with_audio_node(&c.destination())?;
    bind_node(node.port()?.as_ref(), "message", move |e| {
        let d = e.unchecked_into::<MessageEvent>().data();
        let err = get(&d, "error");
        if !err.is_undefined() {
            error(err);
            return;
        }
        let snapshot = get(&d, "snapshot");
        if snapshot.is_undefined() {
            return;
        }
        let copied = APP.with_borrow_mut(|a| {
            let request = get(&d, "request").as_f64().unwrap_or(0.) as u32;
            if a.epoch != epoch || request != a.request {
                return false;
            }
            a.pending = false;
            if a.frozen || doc().hidden() || a.view != a.pending_view {
                return false;
            }
            let array = Float32Array::new(&snapshot);
            if array.length() as usize != SNAPSHOT_LEN {
                return false;
            }
            array.copy_to(&mut a.snapshot);
            a.display.consume(&a.snapshot);
            true
        });
        if copied {
            draw();
        }
    });
    node.port()?.start();
    bind_node(node.as_ref(), "processorerror", |_| {
        APP.with_borrow_mut(|a| {
            a.playing = false;
            if let Some(g) = &a.gain {
                g.gain().set_value(0.);
            }
        });
        error("Audio processor stopped. Power off, then on to restart.".into());
    });
    APP.with_borrow_mut(|a| {
        a.wasm = Some(wasm);
        a.node = Some(node);
        a.gain = Some(gain);
        a.playing = true;
    });
    params();
    if let Some(m) = APP.with_borrow(|a| a.midi.clone()) {
        post("midi", m);
    }
    text("power", "POWER OFF");
    el("power").set_attribute("aria-pressed", "true")?;
    el("power-led").class_list().add_1("on")?;
    el_instrument().class_list().add_1("powered")?;
    el("play").remove_attribute("disabled")?;
    el("stop").remove_attribute("disabled")?;
    let revision = APP.with_borrow(|a| a.source_revision);
    attach_source(epoch, revision).await?;
    if APP.with_borrow(|a| a.epoch != epoch) {
        return Ok(());
    }
    APP.with_borrow_mut(|a| a.busy = false);
    text("play", "PAUSE");
    text("status", "Running. LISTEN enables the monitor output.");
    text(
        "engine-info",
        &format!("{:.0} Hz · FFT 1024 · HOP 256", c.sample_rate()),
    );
    draw();
    Ok(())
}
fn detach() {
    APP.with_borrow_mut(|a| {
        if let Some(n) = a.source.take() {
            let _ = n.disconnect();
            if let Some(b) = n.dyn_ref::<AudioBufferSourceNode>() {
                let scheduled: AudioScheduledSourceNode = b.clone().unchecked_into();
                let _ = scheduled.stop();
            }
            if let Some(o) = n.dyn_ref::<OscillatorNode>() {
                let scheduled: AudioScheduledSourceNode = o.clone().unchecked_into();
                let _ = scheduled.stop();
            }
        }
        if let Some(s) = a.stream.take() {
            for t in s.get_tracks().iter() {
                t.unchecked_into::<MediaStreamTrack>().stop();
            }
        }
    });
    let _ = el("patch-input").class_list().remove_1("connected");
}
fn stop() {
    detach();
    let c = APP.with_borrow_mut(|a| {
        a.epoch = a.epoch.wrapping_add(1);
        a.source_revision = a.source_revision.wrapping_add(1);
        for (t, e, f) in a.node_handlers.drain(..) {
            let _ = t.remove_event_listener_with_callback(e, f.as_ref().unchecked_ref());
        }
        a.display.reset();
        a.snapshot.fill(0.);
        a.node = None;
        a.gain = None;
        a.playing = false;
        a.busy = false;
        a.pending = false;
        a.request = a.request.wrapping_add(1);
        a.context.take()
    });
    if let Some(c) = c {
        let _ = c.close();
    }
    text("power", "POWER ON");
    let _ = el("power").set_attribute("aria-pressed", "false");
    let _ = el("power-led").class_list().remove_1("on");
    let _ = el_instrument().class_list().remove_1("powered");
    let _ = el("patch-output").class_list().remove_1("connected");
    el("play").set_attribute("disabled", "").unwrap();
    el("stop").set_attribute("disabled", "").unwrap();
    text("play", "PLAY");
    text("status", "Power off. Microphone released.");
    draw();
}
fn current_position() -> f64 {
    APP.with_borrow(|a| {
        if a.source.is_none() {
            return a.offset;
        }
        if let Some(c) = &a.context {
            let time = a.offset + c.current_time() - a.started;
            if select("source").value() == "file" {
                if let Some(f) = &a.file {
                    return time.rem_euclid(f.duration());
                }
            }
            time.max(0.)
        } else {
            a.offset
        }
    })
}
async fn attach_source(epoch: u64, revision: u64) -> Result<(), JsValue> {
    detach();
    let (c, node) = APP.with_borrow(|a| (a.context.clone(), a.node.clone()));
    let c = c.ok_or("Power on first.")?;
    let node = node.ok_or("Audio engine is still starting.")?;
    let source = select("source").value();
    if source == "mic" {
        input("monitor").set_checked(false);
        update_monitor();
        let constraints = MediaStreamConstraints::new();
        let options = Object::new();
        for k in ["echoCancellation", "noiseSuppression", "autoGainControl"] {
            set(&options, k, false);
        }
        constraints.set_audio(&options);
        text("status", "Waiting for microphone permission…");
        let stream: MediaStream = JsFuture::from(
            window()
                .unwrap()
                .navigator()
                .media_devices()?
                .get_user_media_with_constraints(&constraints)?,
        )
        .await?
        .dyn_into()?;
        if APP.with_borrow(|a| a.epoch != epoch || a.source_revision != revision)
            || select("source").value() != "mic"
        {
            for t in stream.get_tracks().iter() {
                t.unchecked_into::<MediaStreamTrack>().stop();
            }
            return Ok(());
        }
        let s = c.create_media_stream_source(&stream)?;
        s.connect_with_audio_node(&node)?;
        APP.with_borrow_mut(|a| {
            a.stream = Some(stream);
            a.source = Some(s.unchecked_into());
            a.started = c.current_time();
        });
        post("duration", 0.);
        text("source-name", "MICROPHONE");
    } else if source == "file" {
        let b = APP
            .with_borrow(|a| a.file.clone())
            .ok_or("Load an audio file first.")?;
        let n = c.create_buffer_source()?;
        n.set_buffer(Some(&b));
        n.set_loop(true);
        n.connect_with_audio_node(&node)?;
        let offset = APP.with_borrow(|a| a.offset).rem_euclid(b.duration());
        post("duration", b.duration());
        post("seek", offset);
        n.start_with_when_and_grain_offset(0., offset)?;
        APP.with_borrow_mut(|a| {
            a.started = c.current_time();
            a.offset = offset;
            a.source = Some(n.unchecked_into());
        });
    } else {
        let oscillator = c.create_oscillator()?;
        let mut real = [0.; 4];
        let mut imag = [0., 0.2, 0.07, 0.03];
        let wave = c.create_periodic_wave(&mut real, &mut imag)?;
        oscillator.set_periodic_wave(&wave);
        oscillator.frequency().set_value(value("test-frequency"));
        let test_gain = c.create_gain()?;
        test_gain.gain().set_value(0.25);
        oscillator.connect_with_audio_node(&test_gain)?;
        test_gain.connect_with_audio_node(&node)?;
        oscillator.start()?;
        APP.with_borrow_mut(|a| {
            a.started = c.current_time();
            a.source = Some(oscillator.unchecked_into());
        });
        post("duration", 0.);
        text("source-name", "TEST OSCILLATOR");
    }
    input("seek").set_disabled(source != "file");
    button_bank("data-source", &source);
    let _ = el("patch-input")
        .class_list()
        .toggle_with_force("connected", source != "tone");
    text(
        "transport-name",
        match source.as_str() {
            "file" => "AUDIO FILE / LOOP",
            "mic" => "MICROPHONE / LIVE",
            _ => "TEST OSCILLATOR",
        },
    );
    Ok(())
}
fn retune_test() {
    let f = value("test-frequency");
    APP.with_borrow(|a| {
        if let (Some(n), Some(c)) = (&a.source, &a.context) {
            if let Some(o) = n.dyn_ref::<OscillatorNode>() {
                let _ = o.frequency().set_target_at_time(f, c.current_time(), 0.01);
            }
        }
    });
}
fn restart(reset: bool) {
    if APP.with_borrow(|a| a.node.is_none()) {
        return;
    }
    let (epoch, revision) = APP.with_borrow_mut(|a| {
        a.busy = true;
        a.offset = 0.;
        a.source_revision = a.source_revision.wrapping_add(1);
        (a.epoch, a.source_revision)
    });
    if reset {
        reset_engine();
    }
    spawn_local(async move {
        if let Err(e) = attach_source(epoch, revision).await {
            if APP.with_borrow(|a| a.epoch == epoch && a.source_revision == revision) {
                error(e);
            }
        }
        APP.with_borrow_mut(|a| {
            if a.epoch == epoch && a.source_revision == revision {
                a.busy = false;
            }
        });
    });
}
fn update_monitor() {
    let enabled = input("monitor").checked();
    APP.with_borrow(|a| {
        if let (Some(g), Some(c)) = (&a.gain, &a.context) {
            let _ =
                g.gain()
                    .set_target_at_time(if enabled { 0.5 } else { 0. }, c.current_time(), 0.01);
        }
    });
    let _ =
        el("patch-output").set_attribute("aria-pressed", if enabled { "true" } else { "false" });
    let powered = APP.with_borrow(|a| a.context.is_some());
    let _ = el("patch-output")
        .class_list()
        .toggle_with_force("connected", powered && enabled);
}
fn draw() {
    let captured = APP.with_borrow_mut(|a| {
        a.display
            .draw_aux(&a.snapshot, a.context.is_some(), a.timeline.as_ref());
        a.display.draw(
            &a.snapshot,
            a.view,
            a.context.is_some(),
            a.playing,
            a.frozen,
            a.timeline.as_ref(),
        )
    });
    if captured {
        APP.with_borrow_mut(|a| a.frozen = true);
        el("freeze").set_attribute("aria-pressed", "true").unwrap();
        text("freeze", "SINGLE HELD");
        text("screen-state", "HOLD");
    }
}
#[wasm_bindgen]
pub fn mount() {
    controls::mount();
    for id in ["source", "effect"] {
        el(id).set_attribute("tabindex", "-1").unwrap();
        el(id).set_attribute("aria-hidden", "true").unwrap();
    }
    tips::mount();
    APP.with_borrow_mut(|a| a.display.settings.read_controls());
    params();
    button_bank("data-view", "6");
    draw();
    for id in ["aux-view-0", "aux-view-1", "aux-view-2"] {
        bind(el(id).as_ref(), "change", |_| draw());
    }
    bind(el("power").as_ref(), "click", |_| {
        if APP.with_borrow(|a| a.context.is_some()) {
            stop();
            return;
        }
        if APP.with_borrow(|a| a.busy) {
            return;
        }
        let epoch = APP.with_borrow_mut(|a| {
            a.epoch = a.epoch.wrapping_add(1);
            a.busy = true;
            a.epoch
        });
        text("status", "Starting audio…");
        spawn_local(async move {
            if let Err(e) = start(epoch).await {
                if APP.with_borrow(|a| a.epoch == epoch) {
                    stop();
                    error(e);
                }
            }
        });
    });
    bind(el("play").as_ref(), "click", |_| {
        let (c, playing, missing, epoch, revision) = APP.with_borrow(|a| {
            (
                a.context.clone(),
                a.playing,
                a.source.is_none(),
                a.epoch,
                a.source_revision,
            )
        });
        if let Some(c) = c {
            spawn_local(async move {
                if !playing && missing {
                    if let Err(e) = attach_source(epoch, revision).await {
                        error(e);
                        return;
                    }
                }
                let result = if playing { c.suspend() } else { c.resume() };
                match result {
                    Ok(p) => {
                        if let Err(e) = JsFuture::from(p).await {
                            error(e);
                            return;
                        }
                        APP.with_borrow_mut(|a| a.playing = !playing);
                        text("play", if playing { "PLAY" } else { "PAUSE" });
                        draw();
                    }
                    Err(e) => error(e),
                }
            });
        }
    });
    bind(el("stop").as_ref(), "click", |_| {
        detach();
        APP.with_borrow_mut(|a| {
            a.offset = 0.;
            a.playing = false;
            a.frozen = false;
            a.source_revision = a.source_revision.wrapping_add(1);
            if let Some(c) = &a.context {
                let _ = c.suspend();
            }
        });
        reset_engine();
        input("seek").set_value("0");
        el("freeze").set_attribute("aria-pressed", "false").unwrap();
        text("freeze", "HOLD TRACE");
        text("play", "PLAY");
        text("play-time", "00:00 / 00:00");
        draw();
    });
    bind(el("monitor").as_ref(), "change", |_| update_monitor());
    bind(el("patch-output").as_ref(), "click", |_| {
        input("monitor").set_checked(!input("monitor").checked());
        update_monitor();
    });
    bind(el("patch-input").as_ref(), "click", |_| {
        input("audio-file").click()
    });
    for source in ["tone", "file", "mic"] {
        let target = doc()
            .query_selector(&format!("[data-source='{source}']"))
            .unwrap()
            .unwrap();
        bind(target.as_ref(), "click", move |_| {
            if source == "file" && APP.with_borrow(|a| a.file.is_none()) {
                input("audio-file").click();
                return;
            }
            select("source").set_value(source);
            button_bank("data-source", source);
            restart(true);
        });
    }
    bind(el("source").as_ref(), "change", |_| restart(true));
    for effect in 0..8 {
        let target = doc()
            .query_selector(&format!("[data-effect='{effect}']"))
            .unwrap()
            .unwrap();
        bind(target.as_ref(), "click", move |_| {
            select("effect").set_value(&effect.to_string());
            params();
        });
    }
    for id in ["effect", "key", "scale"] {
        bind(el(id).as_ref(), "change", |_| params());
    }
    for wave in 0..4 {
        let t = doc()
            .query_selector(&format!("[data-carrier='{wave}']"))
            .unwrap()
            .unwrap();
        bind(t.as_ref(), "click", move |_| {
            input("carrier").set_value(&wave.to_string());
            params();
        });
    }
    for scale in ["0", "12"] {
        let target = doc()
            .query_selector(&format!("[data-scale='{scale}']"))
            .unwrap()
            .unwrap();
        bind(target.as_ref(), "click", move |_| {
            input("scale").set_value(scale);
            button_bank("data-scale", scale);
            params();
        });
    }
    for view in 0..9 {
        let target = doc()
            .query_selector(&format!("[data-view='{view}']"))
            .unwrap()
            .unwrap();
        bind(target.as_ref(), "click", move |_| {
            APP.with_borrow_mut(|a| {
                a.view = view;
                a.display.fresh = false;
            });
            button_bank("data-view", &view.to_string());
            draw();
        });
    }
    bind(el("freeze").as_ref(), "click", |_| {
        let f = APP.with_borrow_mut(|a| {
            a.frozen = !a.frozen;
            a.frozen
        });
        el("freeze")
            .set_attribute("aria-pressed", if f { "true" } else { "false" })
            .unwrap();
        text("freeze", if f { "RELEASE HOLD" } else { "HOLD TRACE" });
        draw();
    });
    for mode in 0..3 {
        let target = doc()
            .query_selector(&format!("[data-trigger='{mode}']"))
            .unwrap()
            .unwrap();
        bind(target.as_ref(), "click", move |_| {
            APP.with_borrow_mut(|a| {
                a.display.settings.trigger_mode = mode;
                a.frozen = false;
                if mode == 2 {
                    a.view = 0;
                }
            });
            button_bank("data-trigger", &mode.to_string());
            if mode == 2 {
                button_bank("data-view", "0");
            }
            el("freeze").set_attribute("aria-pressed", "false").unwrap();
            text("freeze", "HOLD TRACE");
            draw();
        });
    }
    bind(el("trigger-source").as_ref(), "click", |_| {
        let ch = APP.with_borrow_mut(|a| {
            a.display.settings.trigger_source ^= 1;
            a.display.settings.trigger_source
        });
        text(
            "trigger-source",
            if ch == 0 { "SOURCE CH1" } else { "SOURCE CH2" },
        );
        el("trigger-source")
            .set_attribute("aria-pressed", if ch == 0 { "false" } else { "true" })
            .unwrap();
        draw();
    });
    bind(el("trigger-slope").as_ref(), "click", |_| {
        let rising = APP.with_borrow_mut(|a| {
            a.display.settings.rising = !a.display.settings.rising;
            a.display.settings.rising
        });
        text("trigger-slope", if rising { "RISING" } else { "FALLING" });
        el("trigger-slope")
            .set_attribute("aria-pressed", if rising { "false" } else { "true" })
            .unwrap();
        draw();
    });
    let checkboxes = doc()
        .query_selector_all(
            ".coupling input, #show-a, #show-b, #env-before, #env-ideal, #env-actual",
        )
        .unwrap();
    for i in 0..checkboxes.length() {
        let e: Element = checkboxes.item(i).unwrap().unchecked_into();
        bind(e.as_ref(), "change", |_| {
            APP.with_borrow_mut(|a| a.display.settings.read_controls());
            draw();
        });
    }
    bind(el("bypass").as_ref(), "click", |_| {
        APP.with_borrow_mut(|a| a.bypass = !a.bypass);
        params();
    });
    bind(el("bypass-now").as_ref(), "click", |_| {
        APP.with_borrow_mut(|a| a.bypass_until = now() + 1000.);
        params();
    });
    bind(el("load-audio").as_ref(), "click", |_| {
        input("audio-file").click()
    });
    bind(el("load-midi").as_ref(), "click", |_| {
        input("midi-file").click()
    });
    bind(el("audio-file").as_ref(), "change", |_| {
        spawn_local(async {
            if let Err(e) = load_audio().await {
                error(e);
            }
        });
    });
    bind(el("midi-file").as_ref(), "change", |_| {
        spawn_local(async {
            if let Err(e) = load_midi().await {
                error(e);
            }
        });
    });
    bind(el("seek").as_ref(), "change", |_| {
        let offset = APP.with_borrow(|a| {
            a.file
                .as_ref()
                .map(|f| f.duration() * input("seek").value_as_number() / 1000.)
        });
        if let Some(offset) = offset {
            APP.with_borrow_mut(|a| a.offset = offset);
            if APP.with_borrow(|a| a.node.is_some()) {
                detach();
                reset_engine();
                let (e, r) = APP.with_borrow(|a| (a.epoch, a.source_revision));
                spawn_local(async move {
                    if let Err(x) = attach_source(e, r).await {
                        error(x);
                    }
                });
            }
        }
    });
    bind(el("help-button").as_ref(), "click", |_| {
        let hidden = el("manual").has_attribute("hidden");
        if hidden {
            el("manual").remove_attribute("hidden").unwrap();
        } else {
            el("manual").set_attribute("hidden", "").unwrap();
        }
        el("help-button")
            .set_attribute("aria-expanded", if hidden { "true" } else { "false" })
            .unwrap();
    });
    let tick = Closure::<dyn FnMut()>::new(|| {
        let expired = APP.with_borrow_mut(|a| {
            if a.bypass_until > 0. && a.bypass_until <= now() {
                a.bypass_until = 0.;
                true
            } else {
                false
            }
        });
        if expired {
            params();
            text("bypass-now", "BYPASS 1 SEC");
        }
        let remaining = APP.with_borrow(|a| a.bypass_until - now());
        if remaining > 0. {
            text("bypass-now", &format!("BYPASS {:.1}s", remaining / 1000.));
        }
        let request = APP.with_borrow_mut(|a| {
            a.ticks = a.ticks.wrapping_add(1);
            let ready =
                a.context.is_some() && a.playing && !a.frozen && !doc().hidden() && !a.pending;
            if ready && (!input("eco").checked() || a.ticks % 2 == 0) {
                a.pending = true;
                a.pending_view = a.view;
                a.request = a.request.wrapping_add(1);
                let mut mask = crate::scope_math::analysis_mask(a.view);
                if !el("analysis-bay").has_attribute("hidden") {
                    for i in 0..3 {
                        mask |=
                            crate::scope_math::analysis_mask(value(&format!("aux-view-{i}")) as u8);
                    }
                }
                Some((mask, a.request))
            } else {
                None
            }
        });
        if let Some((mask, request)) = request {
            let m = message("view", mask as u32);
            set(&m, "request", request);
            APP.with_borrow(|a| {
                if let Some(n) = &a.node {
                    let _ = n.port().unwrap().post_message(&m);
                }
            });
        }
        if !doc().hidden() {
            let time = current_position();
            APP.with_borrow(|a| {
                let total = a
                    .file
                    .as_ref()
                    .filter(|_| select("source").value() == "file")
                    .map(|f| f.duration())
                    .unwrap_or(0.);
                if total > 0. && a.context.is_some() {
                    input("seek").set_value_as_number(time / total * 1000.);
                }
                if a.context.is_some() {
                    text(
                        "play-time",
                        &format!("{} / {}", format_time(time), format_time(total)),
                    );
                }
            });
        }
    });
    window()
        .unwrap()
        .set_interval_with_callback_and_timeout_and_arguments_0(tick.as_ref().unchecked_ref(), 25)
        .unwrap();
    tick.forget();
    bind(window().unwrap().as_ref(), "pagehide", |_| stop());
    bind(window().unwrap().as_ref(), "resize", |_| draw());
    let _ = doc()
        .document_element()
        .unwrap()
        .set_attribute("data-controls-ready-ms", &format!("{:.0}", now()));
    let _ = el("power").remove_attribute("disabled");
    text("status", "Power off. Choose a source and press POWER ON.");
}
fn format_time(seconds: f64) -> String {
    let n = seconds.max(0.) as u64;
    format!("{:02}:{:02}", n / 60, n % 60)
}
async fn load_audio() -> Result<(), JsValue> {
    let Some(file) = input("audio-file").files().and_then(|f| f.get(0)) else {
        return Ok(());
    };
    if file.size() > 512. * 1024. * 1024. {
        return Err(
            "Audio exceeds the 512 MB browser decode limit. Use a shorter or compressed file."
                .into(),
        );
    }
    let ticket = APP.with_borrow_mut(|a| {
        a.audio_ticket += 1;
        a.audio_ticket
    });
    text("status", "Decoding audio locally…");
    let existing = APP.with_borrow(|a| a.context.clone());
    let c = if let Some(c) = &existing {
        c.clone()
    } else {
        let options = AudioContextOptions::new();
        options.set_sample_rate(48000.);
        AudioContext::new_with_context_options(&options)?
    };
    let data = bytes(file.as_ref())
        .await?
        .dyn_into::<js_sys::ArrayBuffer>()?;
    let decoded = JsFuture::from(c.decode_audio_data(&data)?).await;
    if existing.is_none() {
        let _ = c.close();
    }
    let b: AudioBuffer = decoded?.dyn_into()?;
    if APP.with_borrow(|a| a.audio_ticket != ticket) {
        return Ok(());
    }
    if b.duration() <= 0. {
        return Err("The file contains no decodable audio.".into());
    }
    APP.with_borrow_mut(|a| {
        a.file = Some(b);
        a.offset = 0.;
    });
    select("source").set_value("file");
    text("source-name", &file.name());
    el("source-name").set_attribute("title", &file.name())?;
    button_bank("data-source", "file");
    text(
        "status",
        if APP.with_borrow(|a| a.context.is_some()) {
            "Audio loaded. Playing the file on a loop."
        } else {
            "Audio loaded. Power on to play."
        },
    );
    restart(true);
    input("audio-file").set_value("");
    Ok(())
}
async fn load_midi() -> Result<(), JsValue> {
    let Some(file) = input("midi-file").files().and_then(|f| f.get(0)) else {
        return Ok(());
    };
    if file.size() > 8. * 1024. * 1024. {
        return Err("MIDI exceeds the 8 MB timeline limit.".into());
    }
    let ticket = APP.with_borrow_mut(|a| {
        a.midi_ticket += 1;
        a.midi_ticket
    });
    let data = Uint8Array::new(&bytes(file.as_ref()).await?);
    let timeline =
        crate::midi::Timeline::parse(&data.to_vec(), 48000.).map_err(|e| JsValue::from_str(&e))?;
    if APP.with_borrow(|a| a.midi_ticket != ticket) {
        return Ok(());
    }
    let count = timeline.notes.len();
    let data = Uint8Array::from(timeline.encode().as_slice());
    APP.with_borrow_mut(|a| {
        a.midi = Some(data.clone());
        a.timeline = Some(timeline);
    });
    post("midi", data);
    text("midi-name", &file.name());
    el("midi-name").set_attribute("title", &file.name())?;
    text(
        "status",
        &format!(
            "MIDI loaded: {count} notes. MIDI vocoder, harmony and pitch-lock modes follow its timeline."
        ),
    );
    input("midi-file").set_value("");
    draw();
    Ok(())
}
