# Trace snapping

Open a screen’s INSPECT panel and turn on **SNAP**, or press **N**. Moving over a trace shows its name and sample value. CURSORS (**C**) lets you place A/B; left/right arrows step to adjacent visible samples on the selected trace. Turn SNAP off for free coordinates.

Waveforms, spectra, spectral envelopes, pitch histories/targets, cepstra and X–Y traces support snapping, including the COMPARE panels and additional displays. Spectrograms and MIDI scores show **NO TRACE · FREE** and retain axis coordinates. Spectral floors and pitch limits report the displayed value at the clipped boundary; snapping does not increase FFT resolution.

The renderer caches its visible trace vertices only while SNAP is enabled. Pointer movement searches that geometry and redraws the transparent cursor overlay; it does not run DSP, request a new capture, read canvas pixels or change the audio. X–Y uses screen distance; other plots choose each trace’s nearest sample/bin on X, then the closest trace. Hidden channels, clipping, coupling, channel position and zoom are respected. Held A/B retain their captured measurements until moved. A normal trigger waiting without cached snap geometry remains free until the next acquisition.

The waveform now draws every sample in its bounded capture (at most 1024), preserving exact sample positions without an intermediate extrema reducer. Vertical calibration subtracts the position offset so moving a zero line does not change its measured signal level.
