# ColecoVision GX host protocol, version 1

A **host** is a page that embeds a game (the website's `/demo`, `/play`, `/cabinet`, `/dock`; the conformance harness; a partner site). A **game** is the document running the game, usually a Bevy wasm build in a cross-origin iframe, or a page on a TV reached through the Presentation API.

Messages are plain JSON objects sent with `postMessage` (window transport) or as JSON strings over a `PresentationConnection` (cast transport). Every message carries `"v": 1`. Unknown message types and unknown fields are ignored.

The reference implementation is the `gamebient-input` crate plus its `js/gx.js`. Any engine can implement the game side; it is about 40 lines of JavaScript.

## Handshake

1. The game, once its engine is running and input is live, posts `gx:hello` to `window.parent` (target origin `"*"`; the message carries nothing sensitive).
2. The host replies with its own `gx:hello` to the frame, target origin = the game's origin.
3. The game records the reply's `source` and `origin` and from then on posts `gx:event` only there. The game answers a given host's hello **once**; further hellos from the same host only update `hostHasControls`. Hosts likewise should answer a game hello once per frame load, otherwise the two sides would answer each other forever.

Events the game emits before a host has answered (`ready`, the first state transitions) are queued, up to 64, and delivered to the first host that answers. A `gx:hello` that carries `name` is a game hello; the game side ignores those, so a page that receives its own reply is unaffected.

Hosts must not assume the game is listening before its hello arrives: the game document typically loads long before the engine starts (a "Click to Start" gate). A host that loads the frame and wants to talk can send `gx:hello` on the frame's `load` event and again when the game's hello arrives; only the second one is guaranteed to be seen.

## Game → host

```jsonc
{ "type": "gx:hello", "v": 1,
  "name": "Gravestone Gauntlet",   // display name
  "aspect": "16:9",                // canvas aspect the game is authored for
  "hasTouchControls": true }       // game can draw its own touch pad

{ "type": "gx:event", "v": 1, "event": "ready" }                     // engine running
{ "type": "gx:event", "v": 1, "event": "state", "state": "Playing" } // States transition
{ "type": "gx:event", "v": 1, "event": "started" }                   // a run began
{ "type": "gx:event", "v": 1, "event": "gameover" }                  // a run ended
{ "type": "gx:event", "v": 1, "event": "score", "score": 1500 }
{ "type": "gx:event", "v": 1, "event": "paused", "paused": true }     // pause state changed
{ "type": "gx:event", "v": 1, "event": "custom", "name": "lap", "data": { "n": 2 } }
```

`state` values are the game's own state names (`Debug` of its `States` enum). Hosts should treat them as opaque strings and match the documented ones: `StudioLogo`, `Menu`, `HowToPlay`, `Playing`, `GameOver` for template-derived games.

Events are untrusted input to the host. Never award anything server-side from a client-posted `score` without your own verification.

`aspect` is informational. A game with touch controls letterboxes itself inside whatever frame it is given, and its pad lives in those bars, so hosts must keep the frame full-viewport in in-frame mode rather than shrinking it to `aspect`.

## Host → game

```jsonc
{ "type": "gx:hello", "v": 1,
  "hostHasControls": true }        // host draws its own pad; game hides its overlay

{ "type": "gx:input", "v": 1,
  "buttons": 273,                  // bitmask, see below
  "ax": 0.0, "ay": 0.0 }           // optional analog, -1..1, y up-positive

{ "type": "gx:set", "v": 1, "paused": true }   // also "muted": true|false
```

`gx:set` is advisory: the game applies it if it makes sense in its current state (a game only pauses while playing) and reports the outcome with a `paused` event. A host that pauses a game while showing an overlay should resume it when the overlay goes away.

`gx:input` is a **state**, not an event: send the full held set whenever it changes (and it is fine to send it every frame). The game latches any bit that turns on between two of its frames, so a press shorter than a frame still counts. Send `buttons: 0` on blur or when the host stops relaying so nothing stays held.

### Button bits

| Bit | Value | Button | Canon keyboard equivalent |
|---|---|---|---|
| 0 | 1 | Up | ArrowUp |
| 1 | 2 | Down | ArrowDown |
| 2 | 4 | Left | ArrowLeft |
| 3 | 8 | Right | ArrowRight |
| 4 | 16 | A (primary) | Z |
| 5 | 32 | B (secondary) | X |
| 6 | 64 | Start (menu confirm) | Enter |
| 7 | 128 | Select | Shift |
| 8 | 256 | Pause | Escape |

Bits above 8 are reserved and ignored.

## Legacy `keyEvent` (kept indefinitely)

```jsonc
{ "type": "keyEvent", "key": "z", "code": "KeyZ", "eventType": "keydown" }
```

Replayed by the game as a synthetic `KeyboardEvent` on its canvas. Any game that accepts canon keys works with this and nothing else. New hosts should prefer `gx:input`; it does not depend on focus or on synthetic key events.

## Origins

- The game accepts window messages only from allowed origins: the production site and its subdomains, `*.vercel.app` previews, and `localhost` / `127.0.0.1`. Games may add exact extra origins (`GxConfig::extra_host_origins`).
- The game posts events only to the origin that answered `gx:hello`.
- Hosts should post with the game frame's exact origin as `targetOrigin`, never `"*"`, so a navigated frame never receives input.
- Presentation connections have no origin; the user established the cast, so they are trusted.

## Conformance

`harness/index.html` in the `gamebient-input` repo frames any game URL and checks: hello received with the required fields, `ready` event, state events on menu → play, `gx:input` drives the game, legacy `keyEvent` still drives the game, and the frame's own touch overlay hides when the harness claims controls.
