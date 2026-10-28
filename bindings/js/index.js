import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import initWasm, { initSync as initSyncWasm } from './pkg/tomet_cms_js.js';

let initialized = false;

export async function init(moduleOrPath) {
  if (initialized) return;
  if (moduleOrPath !== undefined) {
    await initWasm(moduleOrPath);
    initialized = true;
    return;
  }
  try {
    const wasmUrl = new URL('./pkg/tomet_cms_js_bg.wasm', import.meta.url);
    const bytes = readFileSync(fileURLToPath(wasmUrl));
    await initWasm({ module_or_path: bytes });
    initialized = true;
  } catch (_err) {
    await initWasm();
    initialized = true;
  }
}

export function initSync(module) {
  if (initialized) return;
  if (module !== undefined) {
    initSyncWasm(module);
    initialized = true;
    return;
  }
  const wasmUrl = new URL('./pkg/tomet_cms_js_bg.wasm', import.meta.url);
  const bytes = readFileSync(fileURLToPath(wasmUrl));
  initSyncWasm({ module_or_path: bytes });
  initialized = true;
}

export * from './pkg/tomet_cms_js.js';
export default init;
