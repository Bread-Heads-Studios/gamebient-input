# gamebient-input

The Gamebient control canon for Bevy 0.18 games, plus the web glue that makes a game playable on phones, arcade cabinets, docks and TVs without any code of its own.

```rust
use bevy::prelude::*;
use gamebient_input::prelude::*;

#[derive(States, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
enum GameState { #[default] Menu, Playing, GameOver }

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .init_state::<GameState>()
        .add_plugins(GxInputPlugin::named("My Game"))
        .add_plugins(StateEvents::<GameState>::default())
        .run();
}

fn jump(input: Res<GameInput>) {
    if input.primary_just_pressed { /* A pressed on keyboard, pad, touch or host */ }
}
```

## What you get

- **`GameInput`**, one resource populated every frame in `PreUpdate` from the keyboard, every gamepad, and the virtual source (touch overlay, host relay, Gamepad API). Gameplay reads only this.
- **The canon bindings**, in one tested place: arrows + WASD / D-pad / left stick to move; Z or Space, South or North for A; X or Shift, West or East for B; Enter or pad Start to confirm; Escape or gamepad Start to pause.
- **Frame-coherent edges** computed on the union of sources, with a latch so a tap shorter than a frame still registers and a button is only "released" when no device holds it.
- **On wasm**, via `js/gx.js` shipped by wasm-bindgen as a snippet:
  - the ColecoVision GX **host protocol** (`gx:hello`, `gx:input`, `gx:event`, `gx:set`), see [`docs/host-protocol.md`](docs/host-protocol.md);
  - the legacy **`keyEvent` bridge**, replayed as synthetic keyboard events exactly as before;
  - a **Gamepad API** poller with the standard mapping, so pads work in the browser without a parent page;
  - a **Presentation API** receiver for the TV cast path;
  - a **DOM touch overlay** (D-pad, A, B, Start, Select, Pause) shown on touch devices unless the host says it draws its own pad. Its taps are real user activation inside the game document, so audio unlocks.
- **`StateEvents::<S>`** posts a `state` event to the host on every transition, and the plugin posts `ready` at startup. Write `HostEvent::Started`, `HostEvent::GameOver`, `HostEvent::Score(n)` and `HostEvent::Paused(b)` from your systems for the rest.
- **`HostCommand`** messages (`Pause`, `Resume`, `Mute(bool)`, `Hello`) arrive when a host sends `gx:set` / `gx:hello`; handle the ones that make sense for your game:

```rust
fn apply_host_commands(mut cmds: MessageReader<HostCommand>, mut paused: ResMut<Paused>) {
    for c in cmds.read() {
        match c {
            HostCommand::Pause => paused.0 = true,
            HostCommand::Resume => paused.0 = false,
            HostCommand::Mute(m) => { /* GlobalVolume + AudioSinkPlayback::mute on live sinks */ }
            HostCommand::Hello { .. } => {}
        }
    }
}
```

## Canvas policy

The plugin reads your `Window` and records a `CanvasPolicy`:

- `fit_canvas_to_parent: false` → **pinned**. The backbuffer stays at the configured physical size on every display. On wasm the glue sizes the canvas box to `physical / devicePixelRatio` and letterboxes it with a CSS transform, so a 1280×720 game shades 0.92 MP on a 4K TV *and* on a DPR 3 phone. `CanvasPolicy::PINNED_720P.window("Title")` builds the matching `Window`; spread your own fields over it.
- `fit_canvas_to_parent: true` → **fit**. The canvas tracks its parent at device resolution; the glue does nothing.

`WindowResolution::with_scale_factor_override` never reduces the number of pixels rendered, on web or native. It only changes the logical size. Do not use it as a render scale.

Your loader needs only `<div id="game-container"><canvas id="game"></canvas></div>`; the glue owns the sizing after `init()`.

## Using it in a game

```toml
[dependencies]
gamebient-input = { git = "https://github.com/Bread-Heads-Studios/gamebient-input", tag = "v0.2.0" }
```

The crate depends on `bevy` with `default-features = false` and only the features it needs (`bevy_state`, `keyboard`, `gamepad`, `std`); your game's own feature list drives everything else. On wasm it also needs `wasm-bindgen` at the exact version of your `wasm-bindgen-cli`.

Your `index.html` no longer needs a `message` listener or Presentation code. Keep the click-to-start gate; if a host may drive the game before the engine starts, expose your unlock as `window.__gxUnlock` so the host's first press can trigger it (see the template).

## Conformance harness

`harness/index.html` frames any game URL, exchanges hellos, drives `gx:input` and legacy `keyEvent`, sends `gx:set`, and ticks off the events it sees. Serve it over HTTP (any static server) so origins are real:

```bash
python3 -m http.server 8082 --directory harness
```

`harness/conformance.mjs` automates the same checks in headless Chrome (software WebGL) and exits non-zero on failure. Run it from a game repo after `build_web.sh`:

```bash
GAME_DIST=dist PLAYING_STATE=Playing node ../gamebient-input/harness/conformance.mjs
```

Add `EXPECT_BACKBUFFER=1280x720` to also assert the pinned backbuffer under a real device scale factor (`DEVICE_SCALE_FACTOR`, default 2), and that the canvas was letterboxed to the expected rendered (CSS) box in the viewport — not just that `canvas.width`/`height` happen to match (the HTML default is 300×150, and the pre-`gxPinCanvas` size is the configured 1280×720, so a naive width check can pass before the glue ever runs). Emulated DPR cannot test this; the harness relaunches Chrome with `--force-device-scale-factor`.

It serves `dist/` and the harness on local ports (`GAME_PORT`, `HARNESS_PORT`, `CHROME_PORT` to change), drives the game to `PLAYING_STATE` with alternating `gx:input` and legacy `keyEvent` presses, then checks the touch overlay under touch emulation. Presses are spaced 3 s apart because Bevy clamps the frame delta under software rendering and the template's screen fades gate input.

## Development

```bash
cargo test                                   # unit tests + doctest
cargo clippy --all-targets -- -D warnings
cargo check --target wasm32-unknown-unknown  # compiles the web glue
```
