// gamebient-input web glue. Imported by the crate through wasm-bindgen
// (`#[wasm_bindgen(module = "/js/gx.js")]`); wasm-bindgen ships it next to
// the game as dist/snippets/<crate>/js/gx.js. Everything that touches a
// browser API lives here; Rust polls a few numbers per frame.
//
// Protocol: docs/host-protocol.md. Bit layout must match src/buttons.rs.

const BIT = {
  UP: 1 << 0, DOWN: 1 << 1, LEFT: 1 << 2, RIGHT: 1 << 3,
  A: 1 << 4, B: 1 << 5, START: 1 << 6, SELECT: 1 << 7, PAUSE: 1 << 8,
};
const ALL_BITS = (1 << 9) - 1;
const PROTOCOL_VERSION = 1;
const STICK_DEADZONE = 0.2;

// Hosts allowed to drive this game over postMessage: the ColecoVision GX
// site (production + Vercel previews) and local dev. Exact extra origins
// come from GxConfig::extra_host_origins.
const DEFAULT_HOST_RE =
  /^https:\/\/([a-z0-9-]+\.)*colecovisiongx\.com$|^https:\/\/[a-z0-9-]+\.vercel\.app$|^https?:\/\/(localhost|127\.0\.0\.1)(:\d+)?$/;

// Legacy keyEvent bridge: the synthetic KeyboardEvent goes to winit's
// canvas listener exactly as the old index.html did.
const CANVAS_ID = 'game';

const state = {
  helloJson: '{}',
  extraOrigins: [],
  host: null,            // { post(obj), origin, kind: 'window' | 'presentation' }
  hostHasControls: false,
  touch: 0,              // bits held on the overlay
  hostBits: 0,           // bits held via gx:input
  hostAx: 0, hostAy: 0,
  padBits: 0,            // bits held on Gamepad API pads
  padAx: 0, padAy: 0,
  latched: 0,
  commands: [],
  // Events posted before any host answered hello; flushed on the first
  // answer so `ready` and the boot state transitions are not lost.
  pendingEvents: [],
  overlay: null,
  padSeeded: false,
  prevPadBits: 0,
};

function originAllowed(origin) {
  return DEFAULT_HOST_RE.test(origin) || state.extraOrigins.includes(origin);
}

function latch(prevBits, nextBits) {
  state.latched |= nextBits & ~prevBits & ALL_BITS;
}

// --- Inbound messages -------------------------------------------------------

function synthesizeKey(data) {
  const type = data.eventType === 'keyup' ? 'keyup' : 'keydown';
  const evt = new KeyboardEvent(type, {
    key: data.key || data.code, code: data.code, bubbles: true, cancelable: true,
  });
  window.dispatchEvent(evt);
  const canvas = document.getElementById(CANVAS_ID);
  if (canvas) canvas.dispatchEvent(evt);
}

function handleMessage(data, reply) {
  if (!data || typeof data !== 'object') return;
  switch (data.type) {
    case 'keyEvent':
      if (data.code) synthesizeKey(data);
      return;
    case 'gx:hello': {
      // A game-shaped hello (carries `name`, no `hostHasControls`) is never
      // a host: it is our own reply echoing back in a same-window setup.
      if ('name' in data && !('hostHasControls' in data)) return;
      // Answer a given host once; a host that re-sends hello (e.g. in
      // response to ours) only refreshes hostHasControls. Without this the
      // two sides would answer each other forever.
      const isNew = !state.host || state.host.key !== reply.key;
      state.host = reply;
      state.hostHasControls = !!data.hostHasControls;
      state.commands.push(state.hostHasControls ? 'hello:1' : 'hello:0');
      if (isNew) reply.post(JSON.parse(state.helloJson));
      updateOverlayVisibility();
      if (isNew) {
        const queued = state.pendingEvents;
        state.pendingEvents = [];
        for (const ev of queued) state.host.post(ev);
      }
      return;
    }
    case 'gx:input': {
      const bits = (data.buttons | 0) & ALL_BITS;
      latch(state.hostBits, bits);
      state.hostBits = bits;
      state.hostAx = clampAxis(data.ax);
      state.hostAy = clampAxis(data.ay);
      return;
    }
    case 'gx:set':
      if (typeof data.paused === 'boolean') state.commands.push(data.paused ? 'pause' : 'resume');
      if (typeof data.muted === 'boolean') state.commands.push(data.muted ? 'mute:1' : 'mute:0');
      return;
    default:
      return;
  }
}

function clampAxis(v) {
  const n = Number(v);
  if (!Number.isFinite(n)) return 0;
  return Math.max(-1, Math.min(1, n));
}

function installWindowListener() {
  window.addEventListener('message', (e) => {
    if (!originAllowed(e.origin)) return;
    const source = e.source;
    const origin = e.origin;
    handleMessage(e.data, {
      kind: 'window',
      key: source,
      origin,
      post(obj) {
        try { source && source.postMessage(obj, origin); } catch (_) { /* host gone */ }
      },
    });
  });
}

// Presentation API receiver: this page is on a TV; the phone that started
// the cast sends the same JSON messages as strings over the connection.
function installPresentationReceiver() {
  const recv = navigator.presentation && navigator.presentation.receiver;
  if (!recv) return;
  recv.connectionList.then((list) => {
    const wire = (conn) => {
      const reply = {
        kind: 'presentation',
        key: conn,
        origin: null,
        post(obj) {
          try { conn.send(JSON.stringify(obj)); } catch (_) { /* closed */ }
        },
      };
      conn.addEventListener('message', (e) => {
        let data;
        try { data = typeof e.data === 'string' ? JSON.parse(e.data) : e.data; } catch (_) { return; }
        handleMessage(data, reply);
      });
    };
    list.connections.forEach(wire);
    list.addEventListener('connectionavailable', (e) => wire(e.connection));
  }).catch(() => { /* not a receiver */ });
}

// --- Gamepad API --------------------------------------------------------------

const PAD_BUTTON_BITS = {
  0: BIT.A, 3: BIT.A,        // South / North
  1: BIT.B, 2: BIT.B,        // East / West
  8: BIT.SELECT, 9: BIT.PAUSE,
  12: BIT.UP, 13: BIT.DOWN, 14: BIT.LEFT, 15: BIT.RIGHT,
};

function pollGamepads() {
  if (typeof navigator.getGamepads !== 'function') return;
  let bits = 0, ax = 0, ay = 0, any = false;
  let pads;
  try { pads = navigator.getGamepads(); } catch (_) { return; }
  for (const gp of pads) {
    if (!gp) continue;
    any = true;
    gp.buttons.forEach((b, i) => {
      if (b && b.pressed && PAD_BUTTON_BITS[i]) bits |= PAD_BUTTON_BITS[i];
    });
    const x = gp.axes[0] || 0, y = gp.axes[1] || 0;
    if (Math.abs(x) >= STICK_DEADZONE) ax += x;
    // Browser Y is down-positive; GameInput.move_y is up-positive.
    if (Math.abs(y) >= STICK_DEADZONE) ay -= y;
  }
  if (!any) { state.padBits = 0; state.padAx = 0; state.padAy = 0; state.padSeeded = false; return; }
  // First sight after a pad appears: don't turn a button held from the
  // launching press into a fresh edge.
  if (state.padSeeded) latch(state.prevPadBits, bits);
  state.padSeeded = true;
  state.prevPadBits = bits;
  state.padBits = bits;
  state.padAx = Math.max(-1, Math.min(1, ax));
  state.padAy = Math.max(-1, Math.min(1, ay));
}

// --- Touch overlay --------------------------------------------------------------

const OVERLAY_CSS = `
#gx-pad{position:fixed;inset:0;z-index:2147483000;pointer-events:none;font-family:ui-monospace,Menlo,monospace;-webkit-user-select:none;user-select:none;touch-action:none}
#gx-pad[hidden]{display:none}
#gx-pad .gx-grp{position:absolute;pointer-events:auto}
#gx-pad button{pointer-events:auto;touch-action:none;-webkit-tap-highlight-color:transparent;appearance:none;border:1px solid rgba(232,228,223,.28);background:rgba(20,20,28,.72);color:rgba(232,228,223,.9);font:600 12px/1 ui-monospace,Menlo,monospace;letter-spacing:.08em;text-transform:uppercase;backdrop-filter:blur(6px);display:flex;align-items:center;justify-content:center;padding:0;margin:0}
#gx-pad button.gx-on{background:rgba(0,212,255,.28);border-color:rgba(0,212,255,.7);box-shadow:0 0 14px rgba(0,212,255,.35)}
#gx-pad .gx-dpad{width:150px;height:150px}
#gx-pad .gx-dpad button{position:absolute;width:50px;height:50px;border-radius:10px;font-size:16px}
#gx-pad .gx-up{left:50px;top:0}#gx-pad .gx-down{left:50px;top:100px}#gx-pad .gx-left{left:0;top:50px}#gx-pad .gx-right{left:100px;top:50px}
#gx-pad .gx-act{width:136px;height:136px}
#gx-pad .gx-act button{position:absolute;width:62px;height:62px;border-radius:50%;font-size:16px}
#gx-pad .gx-b{left:0;bottom:6px}#gx-pad .gx-a{right:0;top:6px;border-color:rgba(0,212,255,.55)}
#gx-pad .gx-sys{display:flex;flex-direction:column;gap:8px}
#gx-pad .gx-sys button{min-width:64px;height:34px;border-radius:17px;padding:0 12px;font-size:10px}
#gx-pad .gx-dpad-grp{left:max(env(safe-area-inset-left,12px),12px);bottom:max(env(safe-area-inset-bottom,12px),14px)}
#gx-pad .gx-act-grp{right:max(env(safe-area-inset-right,12px),12px);bottom:max(env(safe-area-inset-bottom,12px),22px)}
#gx-pad .gx-sys-grp{left:50%;transform:translateX(-50%);bottom:max(env(safe-area-inset-bottom,12px),14px)}
@media (orientation:landscape){
#gx-pad .gx-dpad-grp{left:max(env(safe-area-inset-left,12px),12px);top:50%;bottom:auto;transform:translateY(-50%)}
#gx-pad .gx-act-grp{right:max(env(safe-area-inset-right,12px),12px);top:50%;bottom:auto;transform:translateY(-50%)}
#gx-pad .gx-sys-grp{left:auto;right:max(env(safe-area-inset-right,12px),12px);bottom:max(env(safe-area-inset-bottom,8px),8px);transform:none;flex-direction:row}
}`;

const OVERLAY_BUTTONS = [
  ['gx-dpad-grp gx-grp', 'gx-dpad', [['gx-up', BIT.UP, '▲'], ['gx-left', BIT.LEFT, '◀'], ['gx-right', BIT.RIGHT, '▶'], ['gx-down', BIT.DOWN, '▼']]],
  ['gx-act-grp gx-grp', 'gx-act', [['gx-b', BIT.B, 'B'], ['gx-a', BIT.A, 'A']]],
  ['gx-sys-grp gx-grp', 'gx-sys', [['gx-pause', BIT.PAUSE, 'Pause'], ['gx-start', BIT.START, 'Start'], ['gx-select', BIT.SELECT, 'Select']]],
];

function hasTouch() {
  return (navigator.maxTouchPoints || 0) > 0 || 'ontouchstart' in window;
}

function buildOverlay() {
  if (state.overlay || typeof document === 'undefined') return;
  const style = document.createElement('style');
  style.textContent = OVERLAY_CSS;
  document.head.appendChild(style);

  const root = document.createElement('div');
  root.id = 'gx-pad';
  root.setAttribute('aria-hidden', 'true');
  const held = new Map(); // pointerId -> bit

  const press = (btn, bit) => {
    latch(state.touch, state.touch | bit);
    state.touch |= bit;
    btn.classList.add('gx-on');
    if (navigator.vibrate) { try { navigator.vibrate(8); } catch (_) {} }
  };
  const release = (btn, bit) => {
    state.touch &= ~bit;
    btn.classList.remove('gx-on');
  };

  for (const [grpClass, innerClass, buttons] of OVERLAY_BUTTONS) {
    const grp = document.createElement('div');
    grp.className = grpClass;
    const inner = document.createElement('div');
    inner.className = innerClass;
    for (const [cls, bit, label] of buttons) {
      const btn = document.createElement('button');
      btn.type = 'button';
      btn.className = cls;
      btn.textContent = label;
      btn.addEventListener('pointerdown', (e) => {
        e.preventDefault();
        try { btn.setPointerCapture(e.pointerId); } catch (_) {}
        held.set(e.pointerId, bit);
        press(btn, bit);
      });
      const up = (e) => {
        if (!held.has(e.pointerId)) return;
        held.delete(e.pointerId);
        release(btn, bit);
      };
      btn.addEventListener('pointerup', up);
      btn.addEventListener('pointercancel', up);
      btn.addEventListener('lostpointercapture', up);
      btn.addEventListener('contextmenu', (e) => e.preventDefault());
      inner.appendChild(btn);
    }
    grp.appendChild(inner);
    root.appendChild(grp);
  }

  // Failsafe: drop every held button if the page loses visibility so a
  // background tab doesn't leave the player running forever.
  const releaseAll = () => {
    held.clear();
    state.touch = 0;
    root.querySelectorAll('.gx-on').forEach((b) => b.classList.remove('gx-on'));
  };
  window.addEventListener('blur', releaseAll);
  document.addEventListener('visibilitychange', () => { if (document.hidden) releaseAll(); });

  document.body.appendChild(root);
  state.overlay = root;
  updateOverlayVisibility();
}

function updateOverlayVisibility() {
  if (!state.overlay) return;
  state.overlay.hidden = state.hostHasControls;
  if (state.hostHasControls) state.touch = 0;
}

// --- Exports (imported by src/web.rs) --------------------------------------------

export function gxInit(helloJson, extraOrigins) {
  state.helloJson = helloJson || '{}';
  state.extraOrigins = (extraOrigins || '').split(',').map((s) => s.trim()).filter(Boolean);
  installWindowListener();
  installPresentationReceiver();
  if (hasTouch()) buildOverlay();
  // Introduce ourselves to whoever embedded us. The hello carries nothing
  // sensitive, so "*" is acceptable here; events only ever go to the origin
  // that answers.
  if (window.parent && window.parent !== window) {
    try {
      window.parent.postMessage(JSON.parse(state.helloJson), '*');
      console.debug('[gx] hello posted to parent');
    } catch (err) {
      console.debug('[gx] hello to parent failed', err);
    }
  }
}

export function gxPollButtons() {
  pollGamepads();
  return (state.touch | state.hostBits | state.padBits) & ALL_BITS;
}

export function gxTakeLatched() {
  const l = state.latched;
  state.latched = 0;
  return l;
}

export function gxAxisX() {
  return Math.max(-1, Math.min(1, state.hostAx + state.padAx));
}

export function gxAxisY() {
  return Math.max(-1, Math.min(1, state.hostAy + state.padAy));
}

const MAX_PENDING_EVENTS = 64;

export function gxPostEvent(json) {
  let obj;
  try { obj = JSON.parse(json); } catch (_) { return; }
  if (!state.host) {
    // Keep the boot sequence (ready, first state transitions) for whichever
    // host answers; drop the oldest if nobody ever does.
    if (state.pendingEvents.length >= MAX_PENDING_EVENTS) state.pendingEvents.shift();
    state.pendingEvents.push(obj);
    return;
  }
  state.host.post(obj);
}

export function gxTakeCommands() {
  const c = state.commands;
  state.commands = [];
  return c;
}

export function gxHasTouch() {
  return hasTouch();
}
