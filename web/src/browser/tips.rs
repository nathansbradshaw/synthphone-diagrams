use super::*;

fn tip(e: &Element) -> Option<&'static str> {
    let id = e.id();
    let specific = match id.as_str() {
        "power" => "Start or stop audio. Listening stays muted until enabled.",
        "strength" => "Pull your pitch toward the selected scale.",
        "harmony-strength" => "Pull harmony voices toward the MIDI chord.",
        "shift" | "harmony-shift" => "Transpose in semitones. 12 equals one octave.",
        "formant" => "Change vocal size and tone without changing the note.",
        "bits" => "Fewer bits adds grain and digital distortion.",
        "reduction" => "Higher division adds crunchy, aliased high frequencies.",
        "mix" => "Blend processed sound with the aligned original.",
        "speed" => "Higher values follow pitch changes faster.",
        "frequency" => "Set the vocoder's carrier note. MIDI can take over.",
        "test-frequency" => "Tune the built-in test signal.",
        "vertical" | "vertical-out" => "Waveform height. Lower FS/div zooms in.",
        "position-a" | "position-b" => "Move this waveform up or down.",
        "timebase" => "Time across the waveform. Lower values zoom in.",
        "fine-a" | "fine-b" | "fine-time" => {
            "Fine adjustment. Double-click returns to calibrated scale."
        }
        "intensity" => "Adjust trace brightness.",
        "focus" => "Adjust the trace from sharp to soft.",
        "illumination" => "Adjust the grid brightness.",
        "persistence" => "Leave a longer fading trail behind the waveform.",
        "trigger-level" => "Capture when the selected waveform crosses this level.",
        "trigger-source" => "Choose which channel starts waveform capture.",
        "trigger-slope" => "Capture an upward or downward crossing.",
        "freeze" => "Hold every screen together. Audio keeps running.",
        "bypass" => "Compare with the original. Keeps the same audio delay.",
        "bypass-now" => "Hear the original for one second, then return.",
        "load-audio" | "patch-input" | "audio-file" => {
            "Open local audio. It stays in your browser."
        }
        "load-midi" | "midi-file" => "Load notes for the score, harmony and pitch lock.",
        "play" => "Play or pause the current source.",
        "stop" => "Stop and rewind audio and MIDI together.",
        "seek" => "Move audio and MIDI to the same position.",
        "monitor" | "patch-output" => "Listen to the output. Use headphones with a microphone.",
        "eco" => "Update screens 20 times per second. Audio quality stays the same.",
        "key" | "scale" => "Choose the notes used for pitch correction.",
        "env-before" => "Show the original vocal tone envelope.",
        "env-ideal" => "Show the intended envelope after formant shifting.",
        "env-actual" => "Show the envelope measured from actual output.",
        "show-a" | "show-b" => "Show or hide this channel's plotted traces.",
        "analysis-toggle" => "Show three linked screens. Hold freezes them together.",
        "aux-view-0" | "aux-view-1" | "aux-view-2" => {
            "Choose a measurement for this linked screen."
        }
        "help-button" => "Open the short operating guide.",
        _ => "",
    };
    if !specific.is_empty() {
        return Some(specific);
    }
    if let Some(v) = e.get_attribute("data-view") {
        return Some(match v.as_str() {
            "0" => "Compare the original and actual output waveforms.",
            "1" => "See which frequencies make up each signal.",
            "2" => "See frequencies change over time.",
            "3" => "Follow both pitches and the MIDI target.",
            "4" => "Compare original, ideal and measured vocal tone.",
            "5" => "Read MIDI notes, durations and the current playhead.",
            "6" => "Watch eight input/output measurements together.",
            "7" => "See repeating structure and pitch periods.",
            _ => "Plot input against output to compare their relationship.",
        });
    }
    if let Some(v) = e.get_attribute("data-effect") {
        return Some(match v.as_str() {
            "0" => "Unprocessed reference with matched delay.",
            "1" => "Tune your voice toward the selected key and scale.",
            "2" => "Shape a synthesizer carrier with your voice.",
            "3" => "Hear bit depth and rate reduction on their own.",
            "4" => "Sing through up to eight MIDI harmony notes.",
            "5" => "Shift vocal tone while preserving the original pitch.",
            "6" => "Let the highest MIDI note tune the vocoder carrier.",
            _ => "Lock your voice to the highest MIDI note. Holds across rests.",
        });
    }
    if let Some(v) = e.get_attribute("data-trigger") {
        return Some(match v.as_str() {
            "0" => "Keep displaying even when no trigger matches.",
            "1" => "Wait for a matching edge before replacing the waveform.",
            _ => "Capture one matching waveform and hold all screens.",
        });
    }
    if let Some(v) = e.get_attribute("data-source") {
        return Some(match v.as_str() {
            "tone" => "Use the built-in test oscillator.",
            "file" => "Play the loaded audio file on a loop.",
            _ => "Use a microphone. Starts with listening muted.",
        });
    }
    if e.has_attribute("data-carrier") {
        return Some("Choose the vocoder carrier's waveform and tone.");
    }
    if e.get_attribute("name")
        .is_some_and(|n| n.starts_with("coupling-"))
    {
        return Some(match e.get_attribute("value").as_deref() {
            Some("0") => "AC: remove the waveform's DC offset on screen.",
            Some("1") => "DC: show the complete waveform, including offset.",
            _ => "GND: show this channel's zero reference.",
        });
    }
    None
}
pub(super) fn mount() {
    let t = doc().create_element("div").unwrap();
    t.set_id("control-tip");
    t.set_attribute("role", "tooltip").unwrap();
    t.set_attribute("hidden", "").unwrap();
    doc().body().unwrap().append_child(&t).unwrap();
    let controls = doc().query_selector_all("button,input,select").unwrap();
    for i in 0..controls.length() {
        let e: Element = controls.item(i).unwrap().unchecked_into();
        let Some(tip) = tip(&e) else {
            continue;
        };
        e.set_attribute("aria-description", tip).unwrap();
        // Disabled controls still explain themselves when their surrounding
        // physical cap is hovered.
        let target = if e.class_list().contains("knob-input") {
            e.closest(".knob-mount").unwrap().unwrap_or(e.clone())
        } else {
            e.clone()
        };
        for event in ["pointerenter", "focusin"] {
            let anchor = target.clone();
            bind(target.as_ref(), event, move |_| {
                let r = anchor.get_bounding_client_rect();
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
                let left = (r.x() + r.width() / 2. - 120.).clamp(8., (width - 248.).max(8.));
                let top = if r.bottom() + 82. < height {
                    r.bottom() + 8.
                } else {
                    (r.top() - 78.).max(8.)
                };
                text("control-tip", tip);
                el("control-tip")
                    .set_attribute("style", &format!("left:{left}px;top:{top}px"))
                    .unwrap();
                el("control-tip").remove_attribute("hidden").unwrap();
            });
        }
        for event in ["pointerleave", "focusout"] {
            bind(target.as_ref(), event, |_| {
                el("control-tip").set_attribute("hidden", "").unwrap();
            });
        }
    }
    bind(doc().as_ref(), "keydown", |e| {
        let key: KeyboardEvent = e.unchecked_into();
        if key.key() == "Escape" {
            el("control-tip").set_attribute("hidden", "").unwrap();
        }
    });
    bind(doc().as_ref(), "scroll", |_| {
        el("control-tip").set_attribute("hidden", "").unwrap();
    });
}
