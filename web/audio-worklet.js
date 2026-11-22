// AudioWorkletProcessor is a browser-mandated JS interface. This bridge only
// copies PCM and calls the Rust C ABI; no DOM bindings or codecs in this realm.
class DspBench extends AudioWorkletProcessor {
  constructor(options) {
    super();
    this.api = new WebAssembly.Instance(new WebAssembly.Module(options.processorOptions.wasm), {}).exports;
    this.engine = this.api.engine_new(sampleRate);
    this.refresh();
    this.port.onmessage = ({data}) => {
      try {
        if (data.reset) { this.api.engine_free(this.engine); this.engine = this.api.engine_new(sampleRate); this.refresh(); }
        if (data.params) { this.input.set(data.params); this.api.engine_configure(this.engine); }
        if (data.seek !== undefined) this.api.engine_seek(this.engine, data.seek);
        if (data.midi) {
          const ptr = this.api.midi_alloc(data.midi.length);
          new Uint8Array(this.api.memory.buffer, ptr, data.midi.length).set(data.midi);
          if (!this.api.engine_midi(this.engine, ptr, data.midi.length)) throw Error('Invalid MIDI file');
          this.refresh();
        }
        if (data.view !== undefined) {
          const ptr = this.api.engine_analyze(this.engine, data.view);
          this.refresh();
          this.port.postMessage({snapshot: new Float32Array(this.api.memory.buffer, ptr, 3076)});
        }
      } catch(error) { this.port.postMessage({error: String(error)}); }
    };
  }
  refresh() {
    const pointer = this.api.engine_input(this.engine);
    if (this.buffer === this.api.memory.buffer && this.pointer === pointer) return;
    this.buffer = this.api.memory.buffer;
    this.pointer = pointer;
    this.input = new Float32Array(this.buffer, pointer, 128);
    this.output = new Float32Array(this.buffer, this.api.engine_output(this.engine), 128);
  }
  process(inputs, outputs) {
    const input = inputs[0]?.[0], output = outputs[0][0];
    this.refresh();
    if (input) this.input.set(input); else this.input.fill(0);
    this.api.engine_process(this.engine);
    this.refresh();
    output.set(this.output);
    for (let ch = 1; ch < outputs[0].length; ch++) outputs[0][ch].set(output);
    return true;
  }
}
registerProcessor('dsp-bench', DspBench);
