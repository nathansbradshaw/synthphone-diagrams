//! Shared readout formatting; physical control markup is prepared at build time.
pub fn value_label(id: &str, v: f32) -> String {
    match id {
        "key" => [
            "C", "C♯", "D", "D♯", "E", "F", "F♯", "G", "G♯", "A", "A♯", "B",
        ][v as usize % 12]
            .into(),
        "aux-view-0" | "aux-view-1" | "aux-view-2" => match v as u8 {
            1 => "Spectrum",
            2 => "Spectrogram",
            3 => "Pitch",
            4 => "Envelopes",
            5 => "MIDI score",
            7 => "Cepstrum",
            _ => "X–Y",
        }
        .into(),
        "mix" | "strength" | "harmony-strength" => format!("{:.0}%", v * 100.),
        "frequency" | "test-frequency" => format!("{v:.0} Hz"),
        "shift" | "harmony-shift" | "formant" => format!("{v:+.1} st"),
        "bits" => format!("{v:.0} bit"),
        "reduction" => format!("÷{v:.0}"),
        "position-a" | "position-b" => format!("{v:+.2}"),
        "trigger-level" => format!("{v:+.2} FS"),
        "vertical" | "vertical-out" | "timebase" => format!("{v}"),
        _ => format!("{v:.2}"),
    }
}
#[cfg(not(target_arch = "wasm32"))]
pub mod build {
    use super::value_label;
    use std::fmt::Write;
    fn attr<'a>(tag: &'a str, name: &str, default: &'a str) -> &'a str {
        let prefix = format!(" {name}=\"");
        tag.find(&prefix)
            .and_then(|at| tag[at + prefix.len()..].split('"').next())
            .unwrap_or(default)
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
    fn ticks() -> String {
        let mut svg = String::from(
            "<svg width=\"0\" height=\"0\" style=\"position:absolute\" aria-hidden=\"true\"><defs><g id=\"knob-ticks\">",
        );
        for tick in 0..=27 {
            let a = (-135. + tick as f64 * 10.).to_radians();
            let inner = if tick % 3 == 0 { 34. } else { 36. };
            write!(
                svg,
                "<line x1=\"{:.2}\" y1=\"{:.2}\" x2=\"{:.2}\" y2=\"{:.2}\"/>",
                50. + inner * a.sin(),
                50. - inner * a.cos(),
                50. + 38. * a.sin(),
                50. - 38. * a.cos()
            )
            .unwrap();
        }
        svg.push_str("</g></defs></svg>");
        svg
    }
    fn control(tag: &str) -> Result<String, String> {
        let id = attr(tag, "data-id", "");
        if id.is_empty() {
            return Err("Knob missing data-id".into());
        }
        let label = attr(tag, "data-label", id);
        let values = attr(tag, "data-values", "");
        let ticks = attr(tag, "data-ticks", "");
        let minimum = attr(tag, "data-min", "0");
        let step = attr(tag, "data-step", "1");
        let default = attr(tag, "data-default", "0");
        let maximum = if values.is_empty() {
            attr(tag, "data-max", "1").to_string()
        } else {
            (values.split(',').count() - 1).to_string()
        };
        let number = default.parse::<f64>().map_err(|e| e.to_string())?;
        let low = minimum.parse::<f64>().map_err(|e| e.to_string())?;
        let high = maximum.parse::<f64>().map_err(|e| e.to_string())?;
        let angle = (number - low) / (high - low).max(0.001) * 270. - 135.;
        let value = if values.is_empty() {
            number as f32
        } else {
            values
                .split(',')
                .nth(number as usize)
                .unwrap()
                .parse()
                .map_err(|e: std::num::ParseFloatError| e.to_string())?
        };
        let readout = value_label(id, value);
        let mut svg = String::from(
            "<svg class=\"knob-scale\" viewBox=\"0 0 100 100\" aria-hidden=\"true\"><use href=\"#knob-ticks\"/>",
        );
        let labels = ticks.split(',').collect::<Vec<_>>();
        for (i, label) in labels.iter().enumerate() {
            let a = (-135. + 270. * i as f64 / (labels.len() - 1).max(1) as f64).to_radians();
            write!(
                svg,
                "<text x=\"{:.1}\" y=\"{:.1}\">{label}</text>",
                50. + 46. * a.sin(),
                50. - 46. * a.cos()
            )
            .unwrap();
        }
        svg.push_str("</svg>");
        let mut fine = String::new();
        if matches!(id, "vertical" | "vertical-out" | "timebase") {
            let fine_id = match id {
                "vertical" => "fine-a",
                "vertical-out" => "fine-b",
                _ => "fine-time",
            };
            let max = if id == "timebase" { 1. } else { 2. };
            let angle = (1. - 0.5) / (max - 0.5) * 270. - 135.;
            fine = format!(
                "<div class=\"knob-mount fine-knob\" style=\"--angle:{angle}deg\"><div class=\"knob-face\"><span class=\"knob-shaft\"></span><span class=\"fine-mark\">VAR</span><input id=\"{fine_id}\" class=\"knob-input\" type=\"range\" min=\"0.5\" max=\"{max}\" step=\"0.01\" value=\"1\" data-default=\"1\" aria-label=\"{} fine adjustment\" aria-valuetext=\"1.00\" disabled></div><output class=\"sr-only\" id=\"{fine_id}-value\">1.00</output></div>",
                name(id, label)
            );
        }
        Ok(format!(
            "{} style=\"--angle:{angle}deg\"><label class=\"knob-label\" for=\"{id}\">{label}</label><div class=\"knob-face\">{svg}<span class=\"knob-shaft\"></span><input id=\"{id}\" class=\"knob-input\" type=\"range\" min=\"{minimum}\" max=\"{maximum}\" step=\"{step}\" value=\"{default}\" data-default=\"{default}\" data-values=\"{values}\" aria-label=\"{}\" aria-valuetext=\"{readout}\" disabled>{fine}</div><output class=\"knob-value\" id=\"{id}-value\" for=\"{id}\">{readout}</output></div>",
            tag.trim_end_matches('>'),
            name(id, label)
        ))
    }
    pub fn render(html: &str) -> Result<String, String> {
        let mut output = String::with_capacity(html.len() + 20000);
        let mut rest = html;
        while let Some(at) = rest.find("<div class=\"knob-mount ") {
            output.push_str(&rest[..at]);
            rest = &rest[at..];
            let end = rest.find('>').ok_or("Unterminated knob")? + 1;
            if !rest[end..].starts_with("</div>") {
                return Err("Expected an empty knob mount in source".into());
            }
            output.push_str(&control(&rest[..end])?);
            rest = &rest[end + 6..];
        }
        output.push_str(rest);
        Ok(output.replacen("<body>", &format!("<body>{}", ticks()), 1))
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn complete_controls_are_in_first_document() {
            let html = render(include_str!("../index.html")).unwrap();
            assert_eq!(html.matches("<line ").count(), 28);
            assert_eq!(html.matches("id=\"knob-ticks\"").count(), 1);
            assert!(html.contains("id=\"fine-a\""));
            assert!(html.contains("id=\"fine-time\""));
            assert!(html.contains("id=\"test-frequency-value\" for=\"test-frequency\">220 Hz"));
            assert!(html.contains("id=\"aux-view-0-value\" for=\"aux-view-0\">Envelopes"));
            assert!(html.contains("id=\"vertical-value\" for=\"vertical\">0.2"));
        }
    }
}
