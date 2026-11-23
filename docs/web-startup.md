# Web startup

The photographic assets and instrument dimensions stay unchanged. Knob markup, readouts, screen controls and styles arrive in the first document; the browser connects their behavior after loading the UI module. The audio module still loads on Power.

| Startup cost | Previous release | Current |
| --- | ---: | ---: |
| UI WASM, uncompressed | 306,956 B | 271,297 B |
| Blocking stylesheet requests | 2 | 0 |
| Document elements after mounting | 1,263 | 626 |
| Repeated SVG tick lines | 700 | 28 shared lines |
| Six displayed image assets with AVIF | 169,464 B | 169,464 B |
| Audio WASM | 88,268 B | 88,268 B |

This is about 12% less UI code. Transfer compression, network conditions and caching affect elapsed load time; these counts are not a claim that every page load takes a fixed percentage less time. SVG instances reuse the same tick geometry and colors.

The Rust build prepares controls and shares readout formatting with the browser. The UI crate uses size optimization; audio retains its existing build options and identical content hash. JavaScript and WASM keep independent content-fingerprinted URLs. Previously deployed files remain available for cached pages. Power stays disabled until initialization completes; `data-controls-ready-ms` records that completion relative to navigation for debugging.

Cursor movement paints transparent screen overlays instead of redrawing the plots or requesting analysis. Live X/Y readings appear beside the crosshair with the inspector closed, track the current capture and zoom, and scale for small screens. A/B measurements stay in capture coordinates. PNG export composites the overlays with the plots.

Verification covers prepared control defaults, fine knobs, shared tick definitions, complete initial styles/inspection markup, cache independence, optimized WASM builds, native DSP/math tests, browser power/input, pointer drag, live hover, hold/zoom, phone readability and capture export. Asset bytes and the audio binary are unchanged.
