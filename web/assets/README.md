# Instrument component assets

Made for this project using the built-in `image_gen.imagegen` tool, 22 November 2026. These are generated photographic-style components, not photographs of a manufactured SynthPhone oscilloscope. The user's Hameg, Tektronix and Philips photographs guided the physical design; those reference photographs are not redistributed.

The web app uses every asset below. WebP compression/resizing uses `cwebp`; controls, labels, grid and signal traces remain live HTML/Rust/canvas. Knob lighting stays fixed while the index rotates. The casing uses nine-slice scaling so corners, screws, handle and stand retain their material detail as the panel grows.

| Asset | Optimized size | Use |
| --- | --- | --- |
| `enclosure.webp` | ~320 KB | Casing, textured front plate, handle, fasteners and stand |
| `crt.webp` | ~76 KB | Recessed hood, curved glass and reflections |
| `knob.webp` | ~8 KB | Fluted ivory control caps |
| `button.webp` | ~12 KB | Worn rectangular pushbuttons and coupling switches |
| `bnc.webp` | ~12 KB | Metal input/output connectors |
| `power.webp` | ~8 KB | Worn yellow power switch |

## Final prompt set

All requests used transparent backgrounds. Enclosure generation returned an environmental shadow/background that is retained in the nine-slice enclosure asset. No external photographs or trademarks were composited.

**Enclosure:** A photographic cutout of a real used 1980s bench oscilloscope enclosure, wide 3:2 proportions, completely blank uninterrupted front panel with no CRT holes, controls or text. Blue-gray painted steel case, warm off-white aged faceplate, screws, rubber feet, bail stand and handle. Straight-on with slight top and right visible; front plate roughly x3–96%, y9–93%. Soft upper-left workshop light, subtle real wear. Photograph, not CGI. Transparent exterior.

**Knob:** A transparent centered square macro photograph of one aged ivory fluted phenolic rotary knob. No pointer, text or scale. Front-on parallel camera, unmarked rotationally symmetric shape, real dirt and scuffs, diffuse upper-left light, ten percent margin.

**Button:** A transparent landscape macro photograph of one blank aged ivory pushbutton cap with a narrow black recessed socket, width 2.8 times height. Fine scuffs and wear, no text, symbol or logo, upper-left light, eight percent margin.

**CRT:** A 5:4 close-up photograph of a real vintage CRT assembly. Warm ivory bezel, deep black hood, blank blue-green curved glass. No grid, traces or text. Aperture roughly x11–91%, y10–90%. Workshop light, real reflections and scuffs; transparent exterior margin. Photograph, not CGI.

**BNC:** Create a photographic UI component asset: an extreme macro, straight-on orthographic front photograph of ONE real vintage oscilloscope BNC female input socket, used nickel plated circular nut, dark recessed inner ring and small hollow gold center socket. Light tarnish and fine metal scratches, knurled collar, realistic machining and metal reflections. Exact camera axis parallel to connector face, centered square composition, diffuse light from upper left. Transparent background, isolated component, no panel surface, no cable, no text, no markings. 10 percent transparent margin around metal. Physical photograph, not drawing, not CGI.

**Power:** Photographic UI component asset: one real used vintage oscilloscope POWER pushbutton, circular ochre-yellow translucent plastic cap within a dark gray metal circular mounting collar. Straight-on front view, centered square macro photograph, transparent surrounding background, no panel, no writing, no symbols, no illuminated lamp. Fine plastic scratches, slightly dirty recessed edges and aged metal, soft upper-left workshop light. Real object photograph, not CGI. Ten percent transparent margin.
