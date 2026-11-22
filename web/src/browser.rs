use js_sys::{Array, Float32Array, Object, Reflect, Uint8Array};
use std::{cell::RefCell, collections::VecDeque};
use wasm_bindgen::{JsCast, prelude::*};
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::*;
thread_local! { static APP: RefCell<App> = RefCell::new(App::default()); }
struct App {
    context: Option<AudioContext>,
    node: Option<AudioWorkletNode>,
    gain: Option<GainNode>,
    source: Option<AudioNode>,
    stream: Option<MediaStream>,
    file: Option<AudioBuffer>,
    midi: Option<Uint8Array>,
    wasm: Option<JsValue>,
    view: u8,
    frozen: bool,
    bypass: bool,
    busy: bool,
    pending: bool,
    snapshot: [f32; 3076],
    history: VecDeque<(f32, f32)>,
    waterfall: Option<HtmlCanvasElement>,
    offset: f64,
    started: f64,
    playing: bool,
    ticks: u32,
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
            wasm: None,
            view: 0,
            frozen: false,
            bypass: false,
            busy: false,
            pending: false,
            snapshot: [0.; 3076],
            history: VecDeque::with_capacity(240),
            waterfall: None,
            offset: 0.,
            started: 0.,
            playing: false,
            ticks: 0,
            node_handlers: Vec::new(),
        }
    }
}
fn doc() -> Document {
    window().unwrap().document().unwrap()
}
fn el(id: &str) -> Element {
    doc().get_element_by_id(id).unwrap()
}
fn input(id: &str) -> HtmlInputElement {
    el(id).unchecked_into()
}
fn value(id: &str) -> f32 {
    input(id).value_as_number() as f32
}
fn select(id: &str) -> HtmlSelectElement {
    el(id).unchecked_into()
}
fn text(id: &str, s: &str) {
    el(id).set_text_content(Some(s));
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
fn bind(target: &EventTarget, event: &str, f: impl FnMut(Event) + 'static) {
    let c = Closure::<dyn FnMut(Event)>::new(f);
    target
        .add_event_listener_with_callback(event, c.as_ref().unchecked_ref())
        .unwrap();
    c.forget();
}
fn bind_node(target: &EventTarget, event: &'static str, f: impl FnMut(Event) + 'static) {
    let callback = Closure::<dyn FnMut(Event)>::new(f);
    target
        .add_event_listener_with_callback(event, callback.as_ref().unchecked_ref())
        .unwrap();
    APP.with_borrow_mut(|a| a.node_handlers.push((target.clone(), event, callback)));
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
    APP.with_borrow(|a| {
        if let Some(n) = &a.node {
            let _ = n.port().unwrap().post_message(&message(k, v));
        }
    });
}
fn params() {
    let effect = select("effect").value().parse::<f32>().unwrap_or(0.);
    let bypass = APP.with_borrow(|a| a.bypass);
    let values = [
        effect,
        value("strength"),
        value("shift"),
        value("formant"),
        value("bits"),
        value("reduction"),
        value("frequency"),
        value("mix"),
        if bypass { 1. } else { 0. },
        crate::key::library_key(
            select("key").selected_index() as usize,
            select("scale").value() == "12",
        ) as f32,
        value("speed"),
    ];
    post("params", Float32Array::from(values.as_slice()));
    el("pitch-controls").set_attribute("hidden", "").unwrap();
    el("crush-controls").set_attribute("hidden", "").unwrap();
    if effect == 1. {
        el("pitch-controls").remove_attribute("hidden").unwrap();
    }
    if effect == 3. {
        el("crush-controls").remove_attribute("hidden").unwrap();
    }
    for (id, label) in [
        ("mix", format!("{:.0}%", value("mix") * 100.)),
        ("formant", format!("{:.2}×", value("formant"))),
        ("shift", format!("{:+.0} st", value("shift"))),
        ("bits", format!("{:.0} bit", value("bits"))),
        ("reduction", format!("{:.0}×", value("reduction"))),
        ("frequency", format!("{:.0} Hz", value("frequency"))),
        ("strength", format!("{:.0}%", value("strength") * 100.)),
        ("speed", format!("{:.2}", value("speed"))),
    ] {
        text(&format!("{id}-value"), &label);
        input(id).set_attribute("aria-valuetext", &label).unwrap();
    }
    for (id, min, max) in [("mix", 0., 1.), ("formant", 0.5, 2.)] {
        let angle = (value(id) - min) / (max - min) * 270. - 135.;
        el(&format!("{id}-dial"))
            .set_attribute("style", &format!("--angle:{angle}deg"))
            .unwrap();
    }
}
async fn bytes(file: &Blob) -> Result<JsValue, JsValue> {
    JsFuture::from(file.array_buffer()).await
}
async fn start() -> Result<(), JsValue> {
    let options = AudioContextOptions::new();
    options.set_sample_rate(48000.);
    let context = AudioContext::new_with_context_options(&options)?;
    // Resume during the user's click, before fetching or microphone permission.
    let resume = context.resume()?;
    APP.with_borrow_mut(|a| a.context = Some(context.clone()));
    JsFuture::from(resume).await?;
    let cached = APP.with_borrow(|a| a.wasm.clone());
    let wasm = if let Some(w) = cached {
        w
    } else {
        let response: Response = JsFuture::from(window().unwrap().fetch_with_str("engine.wasm"))
            .await?
            .dyn_into()?;
        if !response.ok() {
            return Err("DSP download failed.".into());
        }
        JsFuture::from(response.array_buffer()?).await?
    };
    JsFuture::from(context.audio_worklet()?.add_module("audio-worklet.js")?).await?;
    let options = AudioWorkletNodeOptions::new();
    options.set_number_of_inputs(1);
    options.set_number_of_outputs(1);
    options.set_output_channel_count(&Array::of1(&1.into()));
    options.set_processor_options(Some(&message("wasm", wasm.clone())));
    let node = AudioWorkletNode::new_with_options(&context, "dsp-bench", &options)?;
    let gain = context.create_gain()?;
    gain.gain()
        .set_value(if input("monitor").checked() { 0.5 } else { 0. });
    node.connect_with_audio_node(&gain)?;
    gain.connect_with_audio_node(&context.destination())?;
    bind_node(node.port()?.as_ref(), "message", move |e| {
        let data = e.unchecked_into::<MessageEvent>().data();
        let err = get(&data, "error");
        if !err.is_undefined() {
            error(err);
            return;
        }
        let s = get(&data, "snapshot");
        if s.is_undefined() {
            return;
        }
        APP.with_borrow_mut(|a| {
            a.pending = false;
            if !a.frozen && !doc().hidden() {
                Float32Array::new(&s).copy_to(&mut a.snapshot);
                if a.view == 3 {
                    a.history.push_back((a.snapshot[3072], a.snapshot[3073]));
                    if a.history.len() > 240 {
                        a.history.pop_front();
                    }
                }
            }
        });
        draw();
    });
    node.port()?.start();
    bind_node(node.as_ref(), "processorerror", |_| {
        error("Audio processor stopped. Power off and retry.".into())
    });
    APP.with_borrow_mut(|a| {
        a.wasm = Some(wasm);
        a.node = Some(node);
        a.gain = Some(gain);
        a.playing = true;
        a.offset = 0.;
    });
    params();
    if let Some(m) = APP.with_borrow(|a| a.midi.clone()) {
        post("midi", m);
    }
    attach_source().await?;
    APP.with_borrow_mut(|a| a.busy = false);
    text("power", "Power off");
    el("power-led").class_list().add_1("on")?;
    el("play").remove_attribute("disabled")?;
    text("status", "Running. Enable Listen to hear the output.");
    text(
        "engine-info",
        &format!("{:.0} Hz / FFT 1024 / HOP 256", context.sample_rate()),
    );
    Ok(())
}
fn detach() {
    APP.with_borrow_mut(|a| {
        if let Some(s) = a.source.take() {
            let _ = s.disconnect();
            if let Ok(buffer) = s.dyn_into::<AudioBufferSourceNode>() {
                let scheduled: AudioScheduledSourceNode = buffer.unchecked_into();
                let _ = scheduled.stop();
            }
        }
        if let Some(s) = a.stream.take() {
            for track in s.get_tracks().iter() {
                track.unchecked_into::<MediaStreamTrack>().stop();
            }
        }
    });
}
fn stop() {
    detach();
    let context = APP.with_borrow_mut(|a| {
        for (target, event, callback) in a.node_handlers.drain(..) {
            let _ = target
                .remove_event_listener_with_callback(event, callback.as_ref().unchecked_ref());
        }
        a.node = None;
        a.gain = None;
        a.playing = false;
        a.busy = false;
        a.pending = false;
        a.context.take()
    });
    if let Some(c) = context {
        let _ = c.close();
    }
    text("power", "Power on");
    let _ = el("power-led").class_list().remove_1("on");
    el("play").set_attribute("disabled", "").unwrap();
    text("play", "Pause");
    text("status", "Power off. Microphone released.");
    text("screen-state", "STANDBY");
    text("measurement", "OFF");
    draw();
}
fn make_buffer(context: &AudioContext) -> Result<AudioBuffer, JsValue> {
    let sr = context.sample_rate();
    let buffer = context.create_buffer(1, sr as u32, sr)?;
    let hz = value("frequency");
    let mut pcm = vec![0.; sr as usize];
    // One exact integer-frequency cycle count per second, making looping seamless.
    for (i, x) in pcm.iter_mut().enumerate() {
        let t = i as f32 / sr;
        let p = std::f32::consts::TAU * hz * t;
        *x = 0.2 * (p.sin() + 0.35 * (2. * p).sin() + 0.15 * (3. * p).sin());
    }
    buffer.copy_to_channel(&pcm, 0)?;
    Ok(buffer)
}
async fn attach_source() -> Result<(), JsValue> {
    detach();
    let context = APP
        .with_borrow(|a| a.context.clone())
        .ok_or("Power on first.")?;
    let node = APP.with_borrow(|a| a.node.clone()).unwrap();
    let source = select("source").value();
    if source == "mic" {
        input("monitor").set_checked(false);
        if let Some(g) = APP.with_borrow(|a| a.gain.clone()) {
            g.gain().set_value(0.);
        }
        let constraints = MediaStreamConstraints::new();
        let audio = Object::new();
        for k in ["echoCancellation", "noiseSuppression", "autoGainControl"] {
            set(&audio, k, false);
        }
        constraints.set_audio(&audio);
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
        // If power/source changed while permission was pending, release the stream.
        if APP.with_borrow(|a| a.context.is_none()) || select("source").value() != "mic" {
            for t in stream.get_tracks().iter() {
                t.unchecked_into::<MediaStreamTrack>().stop();
            }
            return Ok(());
        }
        let s = context.create_media_stream_source(&stream)?;
        s.connect_with_audio_node(&node)?;
        APP.with_borrow_mut(|a| {
            a.stream = Some(stream);
            a.source = Some(s.unchecked_into());
        });
        text("source-name", "Microphone · monitoring starts muted");
    } else {
        let buffer = if source == "file" {
            APP.with_borrow(|a| a.file.clone())
                .ok_or("Choose Load audio to select a file.")?
        } else {
            make_buffer(&context)?
        };
        let source_node = context.create_buffer_source()?;
        source_node.set_buffer(Some(&buffer));
        source_node.set_loop(true);
        source_node.connect_with_audio_node(&node)?;
        let offset = if source == "file" {
            APP.with_borrow(|a| a.offset) % buffer.duration()
        } else {
            0.
        };
        if source == "file" {
            post("seek", offset);
        }
        source_node.start_with_when_and_grain_offset(0., offset)?;
        APP.with_borrow_mut(|a| {
            a.started = context.current_time();
            a.source = Some(source_node.unchecked_into());
        });
        if source == "tone" {
            text("source-name", "Harmonic test signal · local audio only");
        }
    }
    input("seek").set_disabled(source != "file");
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
fn restart(reset: bool) {
    if APP.with_borrow(|a| a.busy || a.context.is_none()) {
        return;
    }
    APP.with_borrow_mut(|a| {
        a.busy = true;
        a.offset = 0.;
    });
    if reset {
        post("reset", true);
        params();
        if let Some(m) = APP.with_borrow(|a| a.midi.clone()) {
            post("midi", m);
        }
    }
    spawn_local(async {
        if let Err(e) = attach_source().await {
            error(e);
        }
        APP.with_borrow_mut(|a| a.busy = false);
    });
}
#[wasm_bindgen]
pub fn mount() {
    params();
    draw();
    bind(el("power").as_ref(), "click", |_| {
        if APP.with_borrow(|a| a.busy) {
            return;
        }
        if APP.with_borrow(|a| a.context.is_some()) {
            stop();
            return;
        }
        APP.with_borrow_mut(|a| a.busy = true);
        text("status", "Warming up the instrument…");
        spawn_local(async {
            if let Err(e) = start().await {
                stop();
                error(e);
            }
        });
    });
    bind(el("play").as_ref(), "click", |_| {
        let (ctx, playing) = APP.with_borrow(|a| (a.context.clone(), a.playing));
        if let Some(c) = ctx {
            spawn_local(async move {
                let p = if playing { c.suspend() } else { c.resume() };
                match p {
                    Ok(p) => {
                        if let Err(e) = JsFuture::from(p).await {
                            error(e);
                            return;
                        }
                        APP.with_borrow_mut(|a| a.playing = !playing);
                        text("play", if playing { "Resume" } else { "Pause" });
                        draw();
                    }
                    Err(e) => error(e),
                }
            });
        }
    });
    bind(el("monitor").as_ref(), "change", |_| {
        if let Some(g) = APP.with_borrow(|a| a.gain.clone()) {
            g.gain()
                .set_value(if input("monitor").checked() { 0.5 } else { 0. });
        }
    });
    bind(el("source").as_ref(), "change", |_| restart(true));
    for id in [
        "mix",
        "formant",
        "shift",
        "strength",
        "speed",
        "bits",
        "reduction",
        "frequency",
    ] {
        bind(el(id).as_ref(), "input", move |_| {
            params();
            if id == "frequency" && select("source").value() == "tone" {
                restart(false);
            }
        });
    }
    for id in ["effect", "key", "scale"] {
        bind(el(id).as_ref(), "change", |_| params());
    }
    for view in 0..5 {
        let target = doc()
            .query_selector(&format!("[data-view='{view}']"))
            .unwrap()
            .unwrap();
        bind(target.as_ref(), "click", move |_| {
            APP.with_borrow_mut(|a| {
                a.view = view;
                a.history.clear();
                a.waterfall = None;
            });
            for i in 0..5 {
                let e = doc()
                    .query_selector(&format!("[data-view='{i}']"))
                    .unwrap()
                    .unwrap();
                let _ = e.class_list().toggle_with_force("selected", i == view);
                e.set_attribute("aria-pressed", if i == view { "true" } else { "false" })
                    .unwrap();
            }
            text(
                "view-label",
                [
                    "TIME DOMAIN",
                    "FREQUENCY DOMAIN",
                    "SPECTRAL HISTORY",
                    "PITCH HISTORY",
                    "SPECTRAL ENVELOPE",
                ][view as usize],
            );
            draw();
        });
    }
    bind(el("freeze").as_ref(), "click", |_| {
        let frozen = APP.with_borrow_mut(|a| {
            a.frozen = !a.frozen;
            a.frozen
        });
        el("freeze")
            .set_attribute("aria-pressed", if frozen { "true" } else { "false" })
            .unwrap();
        text(
            "freeze",
            if frozen {
                "Unfreeze trace"
            } else {
                "Freeze trace"
            },
        );
        draw();
    });
    bind(el("bypass").as_ref(), "click", |_| {
        let bypass = APP.with_borrow_mut(|a| {
            a.bypass = !a.bypass;
            a.bypass
        });
        el("bypass")
            .set_attribute("aria-pressed", if bypass { "true" } else { "false" })
            .unwrap();
        params();
    });
    for id in ["timebase", "vertical"] {
        bind(el(id).as_ref(), "change", |_| draw());
    }
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
                .map(|b| b.duration() * value("seek") as f64 / 1000.)
        });
        if let Some(offset) = offset {
            if APP.with_borrow(|a| a.context.is_none() || a.busy) {
                return;
            }
            APP.with_borrow_mut(|a| a.offset = offset);
            detach();
            post("reset", true);
            params();
            if let Some(m) = APP.with_borrow(|a| a.midi.clone()) {
                post("midi", m);
            }
            spawn_local(async {
                if let Err(e) = attach_source().await {
                    error(e);
                }
            });
        }
    });
    let tick = Closure::<dyn FnMut()>::new(|| {
        let hidden = doc().hidden();
        let request = APP.with_borrow_mut(|a| {
            a.ticks = a.ticks.wrapping_add(1);
            let ready = a.context.is_some() && a.playing && !a.frozen && !hidden && !a.pending;
            if ready && (!input("eco").checked() || a.ticks % 2 == 0) {
                a.pending = true;
                Some(a.view)
            } else {
                None
            }
        });
        if let Some(view) = request {
            post("view", view as u32);
        }
        APP.with_borrow(|a| {
            if let (Some(ctx), Some(file)) = (&a.context, &a.file) {
                if select("source").value() == "file" {
                    let position = (a.offset + ctx.current_time() - a.started) % file.duration();
                    input("seek").set_value_as_number(position / file.duration() * 1000.);
                }
            }
        });
    });
    window()
        .unwrap()
        .set_interval_with_callback_and_timeout_and_arguments_0(tick.as_ref().unchecked_ref(), 25)
        .unwrap();
    tick.forget();
    bind(window().unwrap().as_ref(), "pagehide", |_| stop());
    bind(window().unwrap().as_ref(), "resize", |_| draw());
}
async fn load_audio() -> Result<(), JsValue> {
    let Some(file) = input("audio-file").files().and_then(|f| f.get(0)) else {
        return Ok(());
    };
    if file.size() > 40. * 1024. * 1024. {
        return Err("Choose an audio file smaller than 40 MB.".into());
    }
    text("status", "Decoding audio locally…");
    let existing = APP.with_borrow(|a| a.context.clone());
    let context = if let Some(c) = &existing {
        c.clone()
    } else {
        AudioContext::new()?
    };
    let data = bytes(file.as_ref())
        .await?
        .dyn_into::<js_sys::ArrayBuffer>()?;
    let decoded = JsFuture::from(context.decode_audio_data(&data)?).await;
    if existing.is_none() {
        let _ = context.close();
    }
    let buffer: AudioBuffer = decoded?.dyn_into()?;
    if buffer.duration() > 600. {
        return Err("Choose a clip shorter than 10 minutes to keep the bench light.".into());
    }
    APP.with_borrow_mut(|a| {
        a.file = Some(buffer);
        a.offset = 0.;
    });
    select("source").set_value("file");
    text("source-name", &file.name());
    text("status", "Audio loaded. Power on or use the transport.");
    restart(true);
    input("audio-file").set_value("");
    Ok(())
}
async fn load_midi() -> Result<(), JsValue> {
    let Some(file) = input("midi-file").files().and_then(|f| f.get(0)) else {
        return Ok(());
    };
    if file.size() > 2. * 1024. * 1024. {
        return Err("Choose a MIDI file smaller than 2 MB.".into());
    }
    let data = Uint8Array::new(&bytes(file.as_ref()).await?);
    crate::midi::Timeline::parse(&data.to_vec(), 48000.).map_err(|e| JsValue::from_str(&e))?;
    APP.with_borrow_mut(|a| a.midi = Some(data));
    text("midi-name", &file.name());
    restart(true);
    text(
        "status",
        "MIDI loaded. Choose pitch correction, vocoder, or MIDI harmony.",
    );
    input("midi-file").set_value("");
    Ok(())
}
fn context(canvas: &HtmlCanvasElement) -> CanvasRenderingContext2d {
    canvas.get_context("2d").unwrap().unwrap().unchecked_into()
}
fn line(c: &CanvasRenderingContext2d, points: impl Iterator<Item = (f64, f64)>, color: &str) {
    c.set_stroke_style_str(color);
    c.set_line_width(1.5);
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
fn db(x: f32) -> f32 {
    20. * x.max(0.000_001).log10()
}
fn draw() {
    let canvas: HtmlCanvasElement = el("scope").unchecked_into();
    // Fixed logical pixels: retina screens cannot multiply plot/FFT work unboundedly.
    let w = 900.;
    let h = 450.;
    let c = context(&canvas);
    c.clear_rect(0., 0., w, h);
    c.set_stroke_style_str("#365444");
    c.set_line_width(0.6);
    c.begin_path();
    for i in 0..=10 {
        let x = i as f64 * w / 10.;
        c.move_to(x, 0.);
        c.line_to(x, h);
    }
    for i in 0..=8 {
        let y = i as f64 * h / 8.;
        c.move_to(0., y);
        c.line_to(w, y);
    }
    c.stroke();
    c.set_stroke_style_str("#52755e");
    c.begin_path();
    c.move_to(0., h / 2.);
    c.line_to(w, h / 2.);
    c.move_to(w / 2., 0.);
    c.line_to(w / 2., h);
    c.stroke();
    c.set_font("14px monospace");
    APP.with_borrow_mut(|a| {
        let sr = a
            .context
            .as_ref()
            .map(|c| c.sample_rate())
            .unwrap_or(48000.);
        let live = a.context.is_some();
        text(
            "screen-state",
            if a.frozen {
                "HOLD"
            } else if !live {
                "STANDBY"
            } else if !a.playing {
                "PAUSED"
            } else {
                "RUN"
            },
        );
        let snapshot = &a.snapshot;
        text(
            "input-level",
            &if live {
                format!("{:.1} dBFS", db(snapshot[3074]))
            } else {
                "— dBFS".into()
            },
        );
        text(
            "output-level",
            &if live {
                format!("{:.1} dBFS", db(snapshot[3075]))
            } else {
                "— dBFS".into()
            },
        );
        if a.view == 0 {
            let ms = select("timebase").value().parse::<f64>().unwrap_or(2.);
            let count = (ms * 10. * sr as f64 / 1000.).min(1024.) as usize;
            let div = select("vertical").value().parse::<f64>().unwrap_or(0.25);
            for (offset, color) in [(0, "#9eefb6"), (1024, "#ffc677")] {
                let samples = &snapshot[offset..offset + 1024];
                let trigger = samples
                    .windows(2)
                    .take(1024 - count)
                    .position(|p| p[0] < 0. && p[1] >= 0.)
                    .unwrap_or(0);
                line(
                    &c,
                    (0..count).map(|i| {
                        (
                            i as f64 * w / count as f64,
                            (h / 2. - samples[trigger + i] as f64 * h / (8. * div)).clamp(0., h),
                        )
                    }),
                    color,
                );
            }
            text("screen-scale", &format!("{ms} ms / div · {div} V / div"));
        } else if a.view == 1 || a.view == 4 {
            for (offset, color) in [(2048, "#9eefb6"), (2560, "#ffc677")] {
                line(
                    &c,
                    (1..512).map(|i| {
                        let hz = i as f64 * sr as f64 / 1024.;
                        let x = (hz / 20.).ln() / ((sr as f64 / 2.) / 20.).ln() * w;
                        let db = db(snapshot[offset + i] / 256.) as f64;
                        (x, h - (db + 90.).clamp(0., 90.) / 90. * h)
                    }),
                    color,
                );
            }
            c.set_fill_style_str("#98b89d");
            for hz in [100., 1000., 10000.] {
                let x = (hz / 20f64).ln() / ((sr as f64 / 2.) / 20.).ln() * w;
                let _ = c.fill_text(&format!("{hz:.0} Hz"), x + 4., h - 8.);
            }
            text("screen-scale", "20 Hz — Nyquist · −90 to 0 dBFS");
        } else if a.view == 2 {
            if a.waterfall.is_none() {
                let image: HtmlCanvasElement =
                    doc().create_element("canvas").unwrap().unchecked_into();
                image.set_width(600);
                image.set_height(256);
                a.waterfall = Some(image);
            }
            let image = a.waterfall.as_ref().unwrap();
            let heat = context(image);
            let _ = heat
                .draw_image_with_html_canvas_element_and_sw_and_sh_and_dx_and_dy_and_dw_and_dh(
                    image, 1., 0., 599., 256., 0., 0., 599., 256.,
                );
            for y in 0..256 {
                let ch = if y < 128 { 2048 } else { 2560 };
                let fraction = 1. - (y % 128) as f32 / 128.;
                let hz = 20. * ((sr / 2.) / 20.).powf(fraction);
                let bin = (hz * 1024. / sr).clamp(1., 511.) as usize;
                let level = ((db(snapshot[ch + bin] / 256.) + 80.) / 80.).clamp(0., 1.);
                let (r, g, b) = if ch == 2048 {
                    (40. + 110. * level, 60. + 185. * level, 45. + 130. * level)
                } else {
                    (60. + 195. * level, 45. + 150. * level, 20. + 75. * level)
                };
                heat.set_fill_style_str(&format!(
                    "rgb({:.0} {:.0} {:.0})",
                    r * level,
                    g * level,
                    b * level
                ));
                heat.fill_rect(599., y as f64, 1., 1.);
            }
            let _ = c.draw_image_with_html_canvas_element_and_dw_and_dh(image, 0., 0., w, h);
            text("screen-scale", "A above / B below · log frequency · time");
        } else {
            for (ch, color) in [(0, "#9eefb6"), (1, "#ffc677")] {
                let mut points = Vec::new();
                for (i, (a, b)) in a.history.iter().enumerate() {
                    let hz = if ch == 0 { *a } else { *b };
                    if hz > 50. {
                        points.push((
                            i as f64 * w / 240.,
                            h - ((hz as f64 / 55.).log2() / 5.).clamp(0., 1.) * h,
                        ));
                    } else if !points.is_empty() {
                        line(&c, points.drain(..), color);
                    }
                }
                line(&c, points.into_iter(), color);
            }
            for hz in [110., 220., 440., 880.] {
                c.set_fill_style_str("#98b89d");
                let _ = c.fill_text(
                    &format!("{hz:.0} Hz"),
                    6.,
                    h - ((hz / 55f64).log2() / 5.) * h - 5.,
                );
            }
            text("screen-scale", "55 — 1760 Hz · latest 240 readings");
        }
        text(
            "measure-label",
            if a.view == 3 {
                "PITCH · A / B"
            } else {
                "AUDIO ENGINE"
            },
        );
        text(
            "measurement",
            &if a.view == 3 {
                format!("{:.0} / {:.0} Hz", snapshot[3072], snapshot[3073])
            } else if live {
                format!("{:.0} kHz", sr / 1000.)
            } else {
                "OFF".into()
            },
        );
        if !live {
            c.set_fill_style_str("#9abca1");
            c.set_font("18px monospace");
            let _ = c.fill_text("POWER ON TO ACQUIRE", w / 2. - 105., h / 2. - 25.);
        }
    });
}
