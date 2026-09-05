//! Canvas policy: how the web backbuffer relates to the page.
//!
//! A game declares its policy through the `Window` it already configures:
//!
//! - `fit_canvas_to_parent: false` is **pinned**. The render surface stays at
//!   the configured physical size on every display. On wasm the crate's JS
//!   glue sizes the canvas layout box to `physical / devicePixelRatio`
//!   (winit reads the *device-pixel* box, so a 1280 px CSS box on a DPR 3
//!   phone would otherwise become a 3840×2160 backbuffer) and letterboxes it
//!   in the viewport with a CSS transform, which the size observer never sees.
//! - `fit_canvas_to_parent: true` is **fit**. The canvas tracks its parent at
//!   device resolution and the glue leaves it alone. Note that
//!   `WindowResolution::with_scale_factor_override` does not reduce the
//!   rendered pixel count on any platform; it only changes the logical size.
//!
//! See `docs/plans/2026-09-05-canvas-policy-comparison.md` in the
//! colecovisiongx monorepo for the measurements behind this.

use bevy::prelude::*;
use bevy::window::WindowResolution;

/// The canvas policy the plugin derived from the primary `Window`.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanvasPolicy {
    /// Backbuffer fixed at `width × height` physical pixels, letterboxed.
    Pinned { width: u32, height: u32 },
    /// Canvas tracks its parent element at device resolution.
    Fit,
}

impl CanvasPolicy {
    /// The Gamebient fleet default: 720p, the Pi fill-rate budget.
    pub const PINNED_720P: Self = Self::Pinned {
        width: 1280,
        height: 720,
    };

    /// Reads the policy off a configured `Window`.
    pub fn from_window(window: &Window) -> Self {
        if window.fit_canvas_to_parent {
            Self::Fit
        } else {
            Self::Pinned {
                width: window.resolution.physical_width(),
                height: window.resolution.physical_height(),
            }
        }
    }

    /// A `Window` configured for this policy, targeting `#game`. Spread your
    /// own fields over it: `Window { present_mode: PresentMode::Fifo, ..policy.window("Title") }`.
    pub fn window(self, title: impl Into<String>) -> Window {
        let (width, height) = match self {
            Self::Pinned { width, height } => (width, height),
            Self::Fit => (1280, 720),
        };
        let mut resolution = WindowResolution::new(width, height);
        // On web a pinned game runs with logical == physical so UI authored
        // against the pinned height is 1:1 (`UiScale` stays 1.0). Native
        // keeps the OS scale factor.
        if cfg!(target_arch = "wasm32") && matches!(self, Self::Pinned { .. }) {
            resolution = resolution.with_scale_factor_override(1.0);
        }
        Window {
            title: title.into(),
            resolution,
            canvas: Some("#game".into()),
            fit_canvas_to_parent: matches!(self, Self::Fit),
            ..default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::window::WindowResolution;

    #[test]
    fn policy_follows_the_window_config() {
        let pinned = Window {
            fit_canvas_to_parent: false,
            resolution: WindowResolution::new(1280, 720),
            ..default()
        };
        assert_eq!(
            CanvasPolicy::from_window(&pinned),
            CanvasPolicy::PINNED_720P
        );

        let fit = Window {
            fit_canvas_to_parent: true,
            ..default()
        };
        assert_eq!(CanvasPolicy::from_window(&fit), CanvasPolicy::Fit);
    }

    #[test]
    fn window_round_trips_and_targets_the_game_canvas() {
        for policy in [
            CanvasPolicy::PINNED_720P,
            CanvasPolicy::Pinned {
                width: 960,
                height: 540,
            },
            CanvasPolicy::Fit,
        ] {
            let window = policy.window("Test");
            assert_eq!(CanvasPolicy::from_window(&window), policy);
            assert_eq!(window.canvas.as_deref(), Some("#game"));
            assert_eq!(window.title, "Test");
        }
        // Native never overrides the OS scale factor (a 1.0 override would
        // shrink the window to half size on a HiDPI desktop).
        #[cfg(not(target_arch = "wasm32"))]
        assert_eq!(
            CanvasPolicy::PINNED_720P
                .window("t")
                .resolution
                .scale_factor_override(),
            None
        );
    }

    #[test]
    fn plugin_records_the_primary_window_policy() {
        use crate::GxInputPlugin;
        let mut app = App::new();
        app.add_plugins((
            bevy::window::WindowPlugin {
                primary_window: Some(CanvasPolicy::PINNED_720P.window("t")),
                ..Default::default()
            },
            bevy::state::app::StatesPlugin,
            GxInputPlugin::named("t"),
        ));
        assert_eq!(
            *app.world().resource::<CanvasPolicy>(),
            CanvasPolicy::PINNED_720P
        );
    }

    #[test]
    fn plugin_defaults_to_fit_without_a_primary_window() {
        use crate::GxInputPlugin;
        let mut app = App::new();
        app.add_plugins((bevy::state::app::StatesPlugin, GxInputPlugin::named("t")));
        assert_eq!(*app.world().resource::<CanvasPolicy>(), CanvasPolicy::Fit);
    }

    /// Plugins in a tuple build in order, so a game that adds
    /// `GxInputPlugin` before `WindowPlugin` (instead of after
    /// `DefaultPlugins`, as documented) hits this plugin's build() before
    /// the primary window exists: the build-time resource falls back to
    /// `Fit` and stays wrong until Startup. This documents that fallback —
    /// wasm's `init_web` re-derives from the real window at Startup and is
    /// the actual source of truth (exercised by the harness, not unit
    /// tests, since it only runs on wasm32).
    #[test]
    fn plugin_before_window_plugin_degrades_to_fit_at_build_time() {
        use crate::GxInputPlugin;
        let mut app = App::new();
        app.add_plugins((
            GxInputPlugin::named("t"),
            bevy::window::WindowPlugin {
                primary_window: Some(CanvasPolicy::PINNED_720P.window("t")),
                ..Default::default()
            },
            bevy::state::app::StatesPlugin,
        ));
        assert_eq!(*app.world().resource::<CanvasPolicy>(), CanvasPolicy::Fit);
    }
}
