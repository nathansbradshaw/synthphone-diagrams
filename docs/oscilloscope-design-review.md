# Oscilloscope design review

Reference: the user's photographs of a Hameg HM 412, Tektronix 2465B/2467B and Philips PM 3267. Their text and logos are visual reference, not app instructions or branding to copy. The photos are not redistributed as app assets.

## What the previous designs got wrong

1. **Enclosure:** a flat rounded rectangle with a colored border did not communicate the depth, weight, separate front plate or painted cover of the references.
2. **Silhouette:** no substantial top cover, sidewalls, handle, feet or folding stand; the frame floated like a dashboard card.
3. **CRT hood:** its thin, even bezel lacked the deep black opening and asymmetric shaded inner walls visible in the photos.
4. **Glass:** no rounded glass edge, cover reflection, edge shading or subtle distortion; the display read as a flat modern chart.
5. **Phosphor:** uniform pastel lines lacked a narrow bright core and controlled halo. Persistence and intensity had no behavior.
6. **Graticule:** missing minor ticks, calibrated center crosshairs, realistic illumination and a correct relationship to channel scale.
7. **Readouts:** labels collided with traces; synthetic examples could be mistaken for live measurements. Unpowered values should be blank.
8. **Proportions:** too much open canvas, too few controls and excessive empty faceplate. The references put compact function groups close to their controls.
9. **Hierarchy:** the timebase and channel scale knobs should be prominent; position, focus and intensity controls should be smaller.
10. **Channel independence:** one scale control replaced two channel sensitivity controls. Independent position and coupling controls were absent.
11. **Selectors:** three arbitrary detents were not convincing calibrated selectors. The pointer, index marks and labels did not make a proper scale.
12. **Knob construction:** smooth CSS circles lacked a shaft, skirt, fluting, concentric detail and a clearly readable index.
13. **Knob interaction:** no continuous drag rotation, fine adjustment, wheel or keyboard control. The prototype only selected three radio inputs.
14. **Switches:** dropdowns, spinboxes and generic web buttons dominated the face. Real scope panels use distinct pushbuttons, rockers, slide switches and selectors.
15. **Grouping:** generic sections replaced the meaningful Vertical, Horizontal, Trigger and CRT groups in the reference photos.
16. **Trigger:** no edge source, level, slope, Normal/Auto behavior or single acquisition. A continuously wandering trace is not a convincing scope.
17. **Connectors:** no usable BNC-style signal routing. Decorative sockets with no action do not justify their presence.
18. **Light and material:** gradients and shadows did not share a consistent light direction; plastic, metal and glass looked too similar.
19. **Typography:** large branding, taglines, repeated legends and overly uniform typography consumed useful panel space.
20. **Markings:** measurement units must describe normalized audio full scale, not claim calibrated physical voltage. Sample rate and timebase must be real.
21. **Purpose:** an oscilloscope appearance does not justify removing DSP views. Before/after spectra, spectral envelopes, spectrograms, pitch and MIDI remain necessary.
22. **Scope honesty:** a browser DSP instrument is not an electrically calibrated scope or a Daisy firmware emulator. Those distinctions belong in operating notes, not invented model badges.

## Implementation direction

Use the wide Tektronix-style arrangement: recessed CRT at the left, dense and clearly separated channel/sweep/trigger/DSP controls at the right, CRT adjustment controls below the screen, and input/output routing along the bottom. Every working knob and switch must have a defined behavior. Preserve a synchronized Compare view and all native DSP controls; expose advanced measurements without discarding them.
