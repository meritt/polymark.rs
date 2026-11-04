const VERSION = '@VERSION@';
const STATUS = ['ok', 'invalid_utf8', 'invalid_argument', 'internal'];
const INPUTS = ['plain', 'markdown', 'html', 'rich'];
const OUTPUTS = ['plain', 'markdown', 'html', 'rich', 'telegram'];
const OK = 0;
// polymark_report on wasm32: status, then the report as (pointer, length).
const REPORT_SIZE = 12;

export class PolymarkError extends Error {
  constructor(code) {
    super(code);
    this.name = 'PolymarkError';
    this.code = code;
  }
}

export async function load(source = new URL('./polymark.wasm', import.meta.url)) {
  const module = await compile(source);
  const { exports } = await WebAssembly.instantiate(module, {});
  return new Polymark(exports);
}

async function compile(source) {
  if (source instanceof WebAssembly.Module) return source;
  if (typeof source === 'string') source = new URL(source, import.meta.url);
  if (source instanceof URL) {
    if (source.protocol === 'file:') {
      const { readFile } = await import('node:fs/promises');
      return WebAssembly.compile(await readFile(source));
    }
    source = fetch(source);
  }
  if (source instanceof Promise || source instanceof Response) {
    return WebAssembly.compileStreaming(source);
  }
  return WebAssembly.compile(source);
}

const encoder = new TextEncoder();
const decoder = new TextDecoder('utf-8', { fatal: true });

class Polymark {
  #x;

  constructor(exports) {
    this.#x = exports;
    const version = typeof exports.polymark_version === 'function' ? this.#cString(exports.polymark_version()) : '';
    if (version !== VERSION) throw new PolymarkError('version');
    this.version = VERSION;
  }

  transform(text, from, to) {
    const input = INPUTS.indexOf(from);
    const output = OUTPUTS.indexOf(to);
    if (typeof text !== 'string' || input < 0 || output < 0) throw new PolymarkError('invalid_argument');
    if (!text.isWellFormed()) return { error: 'unicode' };
    return this.#report((p, n, out) => this.#x.polymark_transform(p, n, input, output, out), text);
  }

  clean(markup) {
    if (typeof markup !== 'string') throw new PolymarkError('invalid_argument');
    if (!markup.isWellFormed()) return { error: 'unicode' };
    return this.#report((p, n, out) => this.#x.polymark_clean(p, n, out), markup);
  }

  #report(call, text) {
    const x = this.#x;
    const out = x.polymark_alloc(REPORT_SIZE);
    if (out === 0) throw new PolymarkError('internal');
    try {
      this.#with(encoder.encode(text), (p, n) => call(p, n, out));
      // Read only after the call: memory may have grown and detached old views.
      const view = new DataView(x.memory.buffer, out, REPORT_SIZE);
      const status = view.getUint32(0, true);
      if (status !== OK) throw new PolymarkError(STATUS[status] ?? 'internal');
      const report = new Uint8Array(x.memory.buffer, view.getUint32(4, true), view.getUint32(8, true));
      const result = JSON.parse(decoder.decode(report));
      x.polymark_report_free(out);
      return result;
    } finally {
      x.polymark_dealloc(out, REPORT_SIZE);
    }
  }

  #with(bytes, use) {
    const x = this.#x;
    const n = bytes.length;
    const p = x.polymark_alloc(n);
    if (n > 0 && p === 0) throw new PolymarkError('invalid_argument');
    try {
      new Uint8Array(x.memory.buffer, p, n).set(bytes);
      return use(p, n);
    } finally {
      x.polymark_dealloc(p, n);
    }
  }

  #cString(p) {
    const memory = new Uint8Array(this.#x.memory.buffer);
    let end = p;
    while (memory[end] !== 0) end++;
    return decoder.decode(memory.subarray(p, end));
  }
}
