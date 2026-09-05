#!/usr/bin/env node
// Headless conformance run for a game built with gamebient-input.
//
// Serves the game's dist/ and this harness on local ports, launches headless
// Chrome (software WebGL), and checks through the harness page that the game:
//   1. posts gx:hello and answers the host's hello,
//   2. posts `ready` and state events,
//   3. reaches PLAYING_STATE when driven by gx:input and legacy keyEvent
//      presses (alternating, spaced out because Bevy clamps the frame delta
//      under software rendering and screen fades gate input),
//   4. builds its touch overlay under touch emulation and hides it when a
//      host declares its own controls.
//
// Usage (from the game repo, after build_web.sh):
//   GAME_DIST=dist node /path/to/gamebient-input/harness/conformance.mjs
// Env: GAME_DIST (default ./dist), GAME_PORT (8081), HARNESS_PORT (8082),
//      CHROME_PORT (9333), PLAYING_STATE ("Playing"), MAX_PRESSES (8),
//      CHROME (path to the Chrome binary), PRESS_GAP_MS (3000),
//      EXPECT_BACKBUFFER (unset; e.g. 1280x720 to assert the pinned
//      backbuffer under DEVICE_SCALE_FACTOR, default 2).
// Exit code 0 when every check passes.
import { spawn } from 'node:child_process';
import { mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const env = (k, d) => process.env[k] ?? d;
const GAME_DIST = resolve(env('GAME_DIST', 'dist'));
const GAME_PORT = Number(env('GAME_PORT', 8081));
const HARNESS_PORT = Number(env('HARNESS_PORT', 8082));
const CHROME_PORT = Number(env('CHROME_PORT', 9333));
const PLAYING_STATE = env('PLAYING_STATE', 'Playing');
const MAX_PRESSES = Number(env('MAX_PRESSES', 8));
const PRESS_GAP_MS = Number(env('PRESS_GAP_MS', 3000));
const CHROME = env('CHROME', process.platform === 'darwin'
  ? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome'
  : 'google-chrome');
const HARNESS_DIR = dirname(fileURLToPath(import.meta.url));
const GAME_URL = `http://127.0.0.1:${GAME_PORT}/`;
const HARNESS_URL = `http://localhost:${HARNESS_PORT}/`;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const servers = [
  spawn('python3', ['-m', 'http.server', String(GAME_PORT), '--bind', '127.0.0.1', '--directory', GAME_DIST], { stdio: 'ignore' }),
  spawn('python3', ['-m', 'http.server', String(HARNESS_PORT), '--bind', 'localhost', '--directory', HARNESS_DIR], { stdio: 'ignore' }),
];
const CHROME_BASE_ARGS = [
  '--headless=new', `--remote-debugging-port=${CHROME_PORT}`,
  '--use-angle=swiftshader', '--enable-unsafe-swiftshader', '--ignore-gpu-blocklist',
  '--autoplay-policy=no-user-gesture-required', '--no-first-run', '--no-default-browser-check',
];
function launchChrome(extraArgs) {
  return spawn(CHROME, [
    ...CHROME_BASE_ARGS, `--user-data-dir=${mkdtempSync(join(tmpdir(), 'gx-chrome-'))}`, ...extraArgs,
  ], { stdio: 'ignore' });
}
let chrome = launchChrome(['--window-size=1280,800']);

let ws, nextId = 1; const pending = new Map(); const consoleLines = [];
async function connect() {
  for (let i = 0; i < 100; i++) {
    try {
      const v = await fetch(`http://127.0.0.1:${CHROME_PORT}/json/version`).then((r) => r.json());
      ws = new WebSocket(v.webSocketDebuggerUrl);
      await new Promise((res, rej) => { ws.onopen = res; ws.onerror = rej; });
      ws.onmessage = (m) => {
        const msg = JSON.parse(m.data);
        if (msg.id && pending.has(msg.id)) { const { res, rej } = pending.get(msg.id); pending.delete(msg.id); msg.error ? rej(new Error(JSON.stringify(msg.error))) : res(msg.result); }
        else if (msg.method === 'Runtime.consoleAPICalled' && (msg.params.type === 'error' || msg.params.type === 'warning')) consoleLines.push(msg.params.args.map((a) => a.value ?? a.description ?? '').join(' ').slice(0, 300));
        else if (msg.method === 'Runtime.exceptionThrown') consoleLines.push('EXCEPTION ' + (msg.params.exceptionDetails.exception?.description || msg.params.exceptionDetails.text).slice(0, 300));
      };
      return;
    } catch { await sleep(200); }
  }
  throw new Error('chrome did not start');
}
const send = (method, params = {}, sessionId) => new Promise((res, rej) => {
  const id = nextId++; pending.set(id, { res, rej }); ws.send(JSON.stringify({ id, method, params, sessionId }));
});
async function newPage() {
  const { targetId } = await send('Target.createTarget', { url: 'about:blank' });
  const { sessionId } = await send('Target.attachToTarget', { targetId, flatten: true });
  await send('Page.enable', {}, sessionId); await send('Runtime.enable', {}, sessionId);
  return sessionId;
}
async function evaluate(sessionId, expression) {
  const r = await send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true }, sessionId);
  if (r.exceptionDetails) throw new Error('eval: ' + JSON.stringify(r.exceptionDetails.exception?.description || r.exceptionDetails.text));
  return r.result.value;
}
async function navigate(sessionId, url) {
  await send('Page.navigate', { url }, sessionId);
  for (let i = 0; i < 100; i++) { if ((await evaluate(sessionId, 'document.readyState')) === 'complete') return; await sleep(100); }
}
async function waitFor(sessionId, expression, timeoutMs, label) {
  const t0 = Date.now();
  while (Date.now() - t0 < timeoutMs) { if (await evaluate(sessionId, expression)) return true; await sleep(500); }
  console.error(`TIMEOUT waiting for ${label}`); return false;
}

const results = { game: GAME_URL, playingState: PLAYING_STATE };
try {
  await sleep(500);
  await connect();

  // --- Scenario A: harness handshake, events, gx:input + legacy keyEvent ---
  const h = await newPage();
  await navigate(h, HARNESS_URL);
  await evaluate(h, `document.getElementById('url').value = ${JSON.stringify(GAME_URL)}; document.getElementById('load').onclick(); 1`);
  await waitFor(h, "document.getElementById('game').contentWindow !== null", 10000, 'frame');
  await sleep(6000); // wasm download + compile behind the game's loading bar
  const pressBits = "post({type:'gx:input',v:1,buttons:64,ax:0,ay:0}); setTimeout(()=>post({type:'gx:input',v:1,buttons:0,ax:0,ay:0}),150); 1";
  const pressEnter = "post({type:'keyEvent',key:'Enter',code:'Enter',eventType:'keydown'}); setTimeout(()=>post({type:'keyEvent',key:'Enter',code:'Enter',eventType:'keyup'}),150); 1";
  await evaluate(h, pressEnter); // the pre-engine unlock
  results.hello = await waitFor(h, "document.getElementById('c-hello').classList.contains('ok')", 90000, 'gx:hello');
  results.ready = await waitFor(h, "document.getElementById('c-ready').classList.contains('ok')", 15000, 'ready');
  results.stateEvent = await waitFor(h, "document.getElementById('c-state').classList.contains('ok')", 15000, 'state');
  const states = () => evaluate(h, `[...document.querySelectorAll('#log div.in')].map(d=>d.textContent).filter(t=>t.includes('"state":"')).map(t=>t.split('"state":"')[1].split('"')[0]).reverse()`);
  const reached = async () => (await states()).includes(PLAYING_STATE);
  let presses = 0; results.transitionsBy = { gxInput: 0, keyEvent: 0 };
  while (!(await reached()) && presses < MAX_PRESSES) {
    await sleep(PRESS_GAP_MS);
    const before = (await states()).length;
    const useBits = presses % 2 === 0;
    await evaluate(h, useBits ? pressBits : pressEnter);
    presses++;
    await sleep(1500);
    if ((await states()).length > before) results.transitionsBy[useBits ? 'gxInput' : 'keyEvent']++;
  }
  results.states = await states();
  results.playing = await reached();
  results.presses = presses;

  // gx:set pause round-trip: the game should answer with paused events.
  if (results.playing) {
    // Page-side expression: newest-first log lines → latest `paused` value.
    const latestPaused = `[...document.querySelectorAll('#log div.in')].map(d=>d.textContent).filter(t=>t.includes('"event":"paused"')).map(t=>t.includes('"paused":true'))[0]`;
    await evaluate(h, "post({type:'gx:set',v:1,paused:true}); 1");
    const pausedOn = await waitFor(h, `${latestPaused} === true`, 8000, 'paused event (true)');
    await evaluate(h, "post({type:'gx:set',v:1,paused:false}); 1");
    const pausedOff = await waitFor(h, `${latestPaused} === false`, 8000, 'paused event (false)');
    results.pauseCommand = pausedOn && pausedOff;
  }

  // --- Scenario B: touch overlay, game top-level with touch emulation ---
  const g = await newPage();
  await send('Emulation.setTouchEmulationEnabled', { enabled: true, maxTouchPoints: 5 }, g);
  await send('Emulation.setDeviceMetricsOverride', { width: 390, height: 844, deviceScaleFactor: 3, mobile: true }, g);
  await navigate(g, GAME_URL);
  await waitFor(g, "typeof window.__gxUnlock === 'function'", 60000, 'wasm compiled');
  await evaluate(g, "window.__gxUnlock(); 1");
  results.overlay = await waitFor(g, "!!document.getElementById('gx-pad')", 90000, 'touch overlay');
  if (results.overlay) {
    results.overlayVisible = await evaluate(g, "getComputedStyle(document.getElementById('gx-pad')).display !== 'none'");
    results.overlayButtons = await evaluate(g, "document.querySelectorAll('#gx-pad button').length");
    results.overlayPress = await evaluate(g, `(() => { const b = document.querySelector('#gx-pad .gx-a'); b.dispatchEvent(new PointerEvent('pointerdown', {pointerId: 7, bubbles: true})); const on = b.classList.contains('gx-on'); b.dispatchEvent(new PointerEvent('pointerup', {pointerId: 7, bubbles: true})); return on && !b.classList.contains('gx-on'); })()`);
    await evaluate(g, "window.postMessage({type:'gx:hello', v:1, hostHasControls:true}, location.origin); 1");
    await sleep(500);
    results.overlayHiddenByHost = await evaluate(g, "document.getElementById('gx-pad').hidden === true");
  }
  results.consoleErrors = consoleLines.filter((l) => /panicked|EXCEPTION|Failed to/.test(l));

  // --- Scenario C: pinned backbuffer under a REAL device scale factor ---
  // EXPECT_BACKBUFFER=1280x720 asserts canvas.width/height once the engine
  // runs, in a fresh Chrome launched with --force-device-scale-factor.
  // CDP's Emulation.setDeviceMetricsOverride does not reach ResizeObserver's
  // devicePixelContentBoxSize, so it would pass a loader that is broken on
  // real phones; only the launch flag exercises the real path.
  const EXPECT_BACKBUFFER = env('EXPECT_BACKBUFFER', '');
  if (EXPECT_BACKBUFFER) {
    const dsf = Number(env('DEVICE_SCALE_FACTOR', 2));
    chrome.kill('SIGKILL');
    await sleep(500);
    chrome = launchChrome([`--force-device-scale-factor=${dsf}`, '--window-size=1600,1000']);
    await sleep(500);
    await connect();
    const p = await newPage();
    await navigate(p, GAME_URL);
    await waitFor(p, "typeof window.__gxUnlock === 'function'", 60000, 'wasm compiled (dsf run)');
    await evaluate(p, "window.__gxUnlock(); 1");
    await waitFor(p, "(document.getElementById('game')||{}).width > 0", 60000, 'engine surface');
    await sleep(3000); // let any resize feedback settle
    const bb = await evaluate(p, "(() => { const c = document.getElementById('game'); const r = c.getBoundingClientRect(); return { backbuffer: c.width + 'x' + c.height, dpr: window.devicePixelRatio, rendered: Math.round(r.width) + 'x' + Math.round(r.height) }; })()");
    results.backbufferRun = bb;
    results.backbuffer = bb.backbuffer === EXPECT_BACKBUFFER && bb.dpr === dsf;
  }
} catch (e) {
  results.error = String(e);
} finally {
  chrome.kill('SIGKILL');
  for (const s of servers) s.kill('SIGKILL');
}
const required = ['hello', 'ready', 'stateEvent', 'playing', 'pauseCommand', 'overlay', 'overlayVisible', 'overlayPress', 'overlayHiddenByHost'];
if (process.env.EXPECT_BACKBUFFER) required.push('backbuffer');
results.pass = !results.error && required.every((k) => results[k] === true)
  && results.transitionsBy.gxInput > 0 && results.transitionsBy.keyEvent > 0 && results.consoleErrors.length === 0;
console.log(JSON.stringify(results, null, 2));
process.exit(results.pass ? 0 : 1);
