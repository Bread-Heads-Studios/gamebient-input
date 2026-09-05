//! The Gamebient control canon for Bevy games.
//!
//! Add [`GxInputPlugin`] and read [`GameInput`] from gameplay systems. On
//! wasm the plugin also ships the web glue: the ColecoVision GX host
//! protocol (`gx:hello` / `gx:input` / `gx:event`), the legacy `keyEvent`
//! bridge, a Gamepad API poller, a Presentation API receiver and a DOM
//! touch overlay, so a game is playable on phones, cabinets and docks with
//! no code of its own.
//!
//! ```no_run
//! use bevy::prelude::*;
//! use gamebient_input::prelude::*;
//!
//! #[derive(States, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
//! enum GameState { #[default] Menu, Playing }
//!
//! fn main() {
//!     App::new()
//!         .add_plugins(DefaultPlugins)
//!         .init_state::<GameState>()
//!         .add_plugins(GxInputPlugin::named("My Game"))
//!         .add_plugins(StateEvents::<GameState>::default())
//!         .run();
//! }
//!
//! fn jump(input: Res<GameInput>) {
//!     if input.primary_just_pressed { /* ... */ }
//! }
//! ```

pub mod buttons;
pub mod canvas;
pub mod host;
pub mod input;
#[cfg(target_arch = "wasm32")]
pub mod web;

use bevy::prelude::*;

pub use buttons::{Buttons, Edges};
pub use canvas::CanvasPolicy;
pub use host::{GxConfig, HostCommand, HostEvent, StateEvents};
pub use input::{GameInput, VirtualInput};

pub mod prelude {
    pub use crate::{
        Buttons, CanvasPolicy, GameInput, GxConfig, GxInputPlugin, HostCommand, HostEvent,
        StateEvents, VirtualInput,
    };
}

/// Registers `GameInput` collection, the host protocol messages and (on
/// wasm) the web glue.
#[derive(Debug, Clone, Default)]
pub struct GxInputPlugin {
    pub config: GxConfig,
}

impl GxInputPlugin {
    /// Plugin with the given display name and default aspect.
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            config: GxConfig {
                name: name.into(),
                ..Default::default()
            },
        }
    }
}

impl Plugin for GxInputPlugin {
    fn build(&self, app: &mut App) {
        // Derive the canvas policy from the window the game configured (added
        // by DefaultPlugins, so this plugin must come after them). Without a
        // primary window (tests, headless tools) there is nothing to pin.
        let policy = {
            let mut primary = app
                .world_mut()
                .query_filtered::<&Window, With<bevy::window::PrimaryWindow>>();
            primary
                .single(app.world())
                .map(CanvasPolicy::from_window)
                .unwrap_or(CanvasPolicy::Fit)
        };
        app.insert_resource(policy);
        app.insert_resource(self.config.clone())
            .init_resource::<GameInput>()
            .init_resource::<VirtualInput>()
            .init_resource::<input::InputFrame>()
            // After Bevy's input systems: without this ordering, collect_input
            // is order-ambiguous with keyboard/gamepad event processing and
            // press edges become nondeterministic.
            .add_systems(
                PreUpdate,
                input::collect_input.after(bevy::input::InputSystems),
            )
            .add_plugins(host::HostPlugin);
        #[cfg(target_arch = "wasm32")]
        app.add_plugins(web::WebPlugin);
    }
}
