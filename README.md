# Synthphone DSP Viewer

A Rust audio laboratory for exploring the Synthphone signal chain.

**[Open the web instrument](https://synthphone-dsp-viewer.pinteru.chatgpt.site)** (owner-private hosted edition).

The web edition lives in [`web/`](web/README.md): an analog bench instrument with a dual-trace phosphor display, microphone/file/test input, and shared Rust vocal effects. The original `iced` desktop application remains in `src/`.

## Desktop

Requires the sibling [`synthphone_e_vocal_dsp`](https://github.com/nathansbradshaw/synthphone_e_vocal_dsp) checkout.

```sh
cargo run --release
```

See [DESIGN.md](DESIGN.md) for the original desktop design.
