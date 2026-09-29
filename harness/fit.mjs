#!/usr/bin/env node
// Headless check that the touch pad never covers a pinned canvas on a phone.
// Needs no game build: harness/fit.html pins an empty canvas through the glue.
//
// Usage (from the crate root):  node harness/fit.mjs
// Env: FIT_PORT (8083), CHROME_PORT (9334), CHROME (path to the Chrome binary).
// Exit code 0 when every size passes.
import { spawn } from 'node:child_process';
import { mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const env = (k, d) => process.env[k] ?? d;
const FIT_PORT = Number(env('FIT_PORT', 8083));
const CHROME_PORT = Number(env('CHROME_PORT', 9334));
const CHROME = env('CHROME', process.platform === 'darwin'
  ? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome'
  : 'google-chrome');
const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const PHONE = { width: 390, height: 844 };
const LANDSCAPE = { width: 844, height: 390 };
const SIZES = [[1280, 720], [960, 720], [720, 720], [720, 960]];

const server = spawn('python3', ['-m', 'http.server', String(FIT_PORT), '--bind', '127.0.0.1', '--directory', ROOT], { stdio: 'ignore' });
const chrome = spawn(CHROME, [
  '--headless=new', `--remote-debugging-port=${CHROME_PORT}`, '--no-first-run', '--no-default-browser-check',
  `--user-data-dir=${mkdtempSync(join(tmpdir(), 'gx-fit-'))}`, '--window-size=1280,900',
], { stdio: 'ignore' });

let ws, nextId = 1; const pending = new Map();
async function connect() {
  for (let i = 0; i < 100; i++) {
    try {
      const v = await fetch(`http://127.0.0.1:${CHROME_PORT}/json/version`).then((r) => r.json());
      ws = new WebSocket(v.webSocketDebuggerUrl);
      await new Promise((res, rej) => { ws.onopen = res; ws.onerror = rej; });
      ws.onmessage = (m) => {
        const msg = JSON.parse(m.data);
        if (msg.id && pending.has(msg.id)) {
          const { res, rej } = pending.get(msg.id); pending.delete(msg.id);
          msg.error ? rej(new Error(JSON.stringify(msg.error))) : res(msg.result);
        }
      };
      return;
    } catch { await sleep(200); }
  }
  throw new Error('chrome did not start');
}
const send = (method, params = {}, sessionId) => new Promise((res, rej) => {
  const id = nextId++; pending.set(id, { res, rej }); ws.send(JSON.stringify({ id, method, params, sessionId }));
});
async function evaluate(sessionId, expression) {
  const r = await send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true }, sessionId);
  if (r.exceptionDetails) throw new Error('eval: ' + JSON.stringify(r.exceptionDetails.exception?.description || r.exceptionDetails.text));
  return r.result.value;
}

const MEASURE = `(() => {
  const c = document.getElementById('game').getBoundingClientRect();
  const pads = [...document.querySelectorAll('#gx-pad .gx-grp')].map((g) => g.getBoundingClientRect());
  const hits = pads.filter((p) => p.left < c.right && p.right > c.left && p.top < c.bottom && p.bottom > c.top);
  return { canvas: [c.left, c.top, c.width, c.height].map(Math.round), pads: pads.length, overlapping: hits.length };
})()`;

async function measure(viewport, w, h) {
  const { targetId } = await send('Target.createTarget', { url: 'about:blank' });
  const { sessionId } = await send('Target.attachToTarget', { targetId, flatten: true });
  await send('Page.enable', {}, sessionId); await send('Runtime.enable', {}, sessionId);
  await send('Emulation.setTouchEmulationEnabled', { enabled: true, maxTouchPoints: 5 }, sessionId);
  await send('Emulation.setDeviceMetricsOverride', { ...viewport, deviceScaleFactor: 3, mobile: true }, sessionId);
  await send('Page.navigate', { url: `http://127.0.0.1:${FIT_PORT}/harness/fit.html?w=${w}&h=${h}` }, sessionId);
  for (let i = 0; i < 50; i++) {
    if (await evaluate(sessionId, 'window.__fitReady === true && !!document.getElementById("gx-pad")')) break;
    await sleep(100);
  }
  await sleep(200);
  const m = await evaluate(sessionId, MEASURE);
  await send('Target.closeTarget', { targetId });
  return m;
}

const results = []; let error = null;
try {
  await sleep(500);
  await connect();
  for (const [w, h] of SIZES) {
    const m = await measure(PHONE, w, h);
    const [, top, cw, ch] = m.canvas;
    // Worked out here rather than imported from the glue, so a wrong rule
    // in gxComputeFit cannot agree with itself.
    const full = Math.min(PHONE.width / w, PHONE.height / h);
    const intrudes = (PHONE.height + h * full) / 2 > PHONE.height - 190;
    const s = intrudes ? Math.min(PHONE.width / w, (PHONE.height - 190) / h) : full;
    const expectW = Math.round(w * s);
    const expectTop = intrudes ? 0 : Math.round((PHONE.height - h * s) / 2);
    const ratioOk = Math.abs(cw / ch - w / h) < 0.01;
    const placedOk = Math.abs(cw - expectW) <= 1 && Math.abs(top - expectTop) <= 1;
    const ok = m.pads === 3 && m.overlapping === 0 && ratioOk && placedOk;
    results.push({ viewport: 'portrait', size: `${w}x${h}`, ...m, expectW, expectTop, ok });
  }
  // Landscape: nothing is reserved, so the canvas is centred at the largest fit.
  for (const [w, h] of SIZES) {
    const m = await measure(LANDSCAPE, w, h);
    const [, , cw, ch] = m.canvas;
    const s = Math.min(LANDSCAPE.width / w, LANDSCAPE.height / h);
    const ok = Math.abs(cw - Math.round(w * s)) <= 1 && Math.abs(ch - Math.round(h * s)) <= 1;
    results.push({ viewport: 'landscape', size: `${w}x${h}`, ...m, ok });
  }
} catch (e) {
  error = String(e);
} finally {
  chrome.kill('SIGKILL');
  server.kill('SIGKILL');
}
const pass = !error && results.length === SIZES.length * 2 && results.every((r) => r.ok);
console.log(JSON.stringify({ pass, error, results }, null, 2));
process.exit(pass ? 0 : 1);
