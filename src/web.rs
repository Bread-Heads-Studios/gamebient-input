//! wasm32 glue: bridges `js/gx.js` (the browser side of the host protocol,
//! the legacy `keyEvent` bridge, the Gamepad API poller, the Presentation
//! receiver and the DOM touch overlay) to [`VirtualInput`] and the host
//! messages.
//!
//! JS owns every browser API; Rust only polls a few numbers each frame and
//! hands back JSON strings to post. `js/gx.js` is shipped by wasm-bindgen as
//! a local snippet (`dist/snippets/…/js/gx.js`), so games need no extra
//! build step.

use bevy::prelude::*;
use wasm_bindgen::prelude::*;

use crate::buttons::Buttons;
use crate::canvas::CanvasPolicy;
use crate::host::{GxConfig, HostCommand, HostEvent, encode_event, encode_hello};
use crate::input::{VirtualInput, collect_input};

#[wasm_bindgen(module = "/js/gx.js")]
extern "C" {
    /// Installs listeners, posts the initial `gx:hello`, and builds the
    /// touch overlay when appropriate. `extra_origins` is a comma-separated
    /// list of exact origins accepted on top of the built-in pattern.
    #[wasm_bindgen(js_name = gxInit)]
    fn gx_init(hello_json: &str, extra_origins: &str);

    /// Union of touch overlay, `gx:input` and Gamepad API buttons, as the
    /// protocol bitmask. Polls gamepads as a side effect.
    #[wasm_bindgen(js_name = gxPollButtons)]
    fn gx_poll_buttons() -> u32;

    /// Bits pressed since the previous take (sub-frame taps), then cleared.
    #[wasm_bindgen(js_name = gxTakeLatched)]
    fn gx_take_latched() -> u32;

    #[wasm_bindgen(js_name = gxAxisX)]
    fn gx_axis_x() -> f32;

    #[wasm_bindgen(js_name = gxAxisY)]
    fn gx_axis_y() -> f32;

    /// Posts a `gx:event` JSON string to the host that answered hello.
    #[wasm_bindgen(js_name = gxPostEvent)]
    fn gx_post_event(json: &str);

    /// Host commands received since the previous take, as short strings:
    /// `hello:1|0`, `pause`, `resume`, `mute:1|0`.
    #[wasm_bindgen(js_name = gxTakeCommands)]
    fn gx_take_commands() -> js_sys::Array;

    /// Whether the device has a touch screen (so hello can say so).
    #[wasm_bindgen(js_name = gxHasTouch)]
    fn gx_has_touch() -> bool;

    /// Pins the canvas layout box to `width/DPR × height/DPR` CSS px and
    /// letterboxes it with a transform, so the backbuffer stays `width ×
    /// height` on every display. See `canvas.rs`.
    #[wasm_bindgen(js_name = gxPinCanvas)]
    fn gx_pin_canvas(width: u32, height: u32);
}

/// Registers the web glue systems. Added by `GxInputPlugin` on wasm32.
pub struct WebPlugin;

impl Plugin for WebPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, init_web)
            .add_systems(
                PreUpdate,
                (poll_virtual, poll_commands)
                    .after(bevy::input::InputSystems)
                    .before(collect_input),
            )
            .add_systems(Last, post_events);
    }
}

fn init_web(config: Res<GxConfig>, policy: Res<CanvasPolicy>) {
    let hello = encode_hello(&config, gx_has_touch());
    gx_init(&hello, &config.extra_host_origins.join(","));
    if let CanvasPolicy::Pinned { width, height } = *policy {
        gx_pin_canvas(width, height);
    }
}

fn poll_virtual(mut virt: ResMut<VirtualInput>) {
    virt.set_held(Buttons(gx_poll_buttons()));
    virt.latched |= Buttons(gx_take_latched());
    virt.axis = Vec2::new(gx_axis_x(), gx_axis_y());
}

fn poll_commands(mut out: MessageWriter<HostCommand>) {
    for v in gx_take_commands().iter() {
        let Some(s) = v.as_string() else { continue };
        if let Some(cmd) = parse_command(&s) {
            out.write(cmd);
        }
    }
}

/// Decodes the short command strings the JS side queues.
pub(crate) fn parse_command(s: &str) -> Option<HostCommand> {
    match s {
        "pause" => Some(HostCommand::Pause),
        "resume" => Some(HostCommand::Resume),
        "mute:1" => Some(HostCommand::Mute(true)),
        "mute:0" => Some(HostCommand::Mute(false)),
        "hello:1" => Some(HostCommand::Hello {
            host_has_controls: true,
        }),
        "hello:0" => Some(HostCommand::Hello {
            host_has_controls: false,
        }),
        _ => None,
    }
}

fn post_events(mut events: MessageReader<HostEvent>) {
    for e in events.read() {
        gx_post_event(&encode_event(e));
    }
}
