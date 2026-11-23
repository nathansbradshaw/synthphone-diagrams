# Web startup optimization

The initial instrument transferred large photographic assets and fetched UI WebAssembly only after the JavaScript bootstrap arrived. A single build-wide query version also invalidated unchanged code after CSS-only updates.

| Resource | Before | After |
| --- | ---: | ---: |
| Enclosure image | 324,830 B WebP | 106,599 B AVIF |
| CRT image | 77,252 B WebP | 28,883 B AVIF |
| All six displayed image assets | 436,064 B | 169,464 B with AVIF |
| UI WASM, uncompressed | 464,916 B | 257,168 B |
| Audio WASM, uncompressed | 112,419 B | 88,268 B |

The raw UI WASM reduction is about 45%; HTTP gzip reduces both versions further, so the transfer saving for WASM is smaller. Initial compressed HTML/CSS/JS/WASM plus preferred images is roughly half the previous transfer budget. This is a payload comparison, not a claim that every connection loads twice as fast. One sampled public enclosure download took 10.4 seconds; network timing varies.

The page starts UI JavaScript, UI WASM and large image requests from the document head. AVIF-capable browsers use the smaller images; WebP remains available. Binaryen's size optimizer runs without fast-math options. Every code/style asset receives its own content-fingerprinted filename; editing CSS no longer changes an unchanged module's URL. Prior deployed files are retained so older cached pages remain consistent.

Power becomes available when its controls are connected. The document's `data-controls-ready-ms` records actual initialization completion relative to navigation for debugging; it is not shown in the instrument. Local preview initialization measured 44 ms on the development machine; this is not a public-network benchmark.

Verification covers content-based cache independence, local WASM builds, the eight processor paths in the browser, live A3 input/output measurement, power cycling, AVIF appearance and absence of browser errors. Native DSP tests remain applicable to the source; the browser checks exercise the optimized audio module.
