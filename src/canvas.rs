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
    /// Legacy 16:9. The largest sanctioned size: 0.92 MP, the Pi fill-rate
    /// budget.
    pub const PINNED_720P: Self = Self::Pinned {
        width: 1280,
        height: 720,
    };

    /// 4:3 landscape, the template default.
    pub const PINNED_4X3: Self = Self::Pinned {
        width: 960,
        height: 720,
    };

    /// 1:1 square. Looks the same on horizontal and vertical cabinets.
    pub const PINNED_1X1: Self = Self::Pinned {
        width: 720,
        height: 720,
    };

    /// 3:4 portrait.
    pub const PINNED_3X4: Self = Self::Pinned {
        width: 720,
        height: 960,
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

    /// The policy to act on at `Startup`, given the one recorded at plugin
    /// build and the live primary window.
    ///
    /// A `Pinned` size recorded at build is the size the game configured and
    /// always wins. By `Startup`, winit may have resized the window to the
    /// canvas's device-pixel box (a 960 px CSS box at DPR 2 reads as 1920),
    /// so the live window cannot be trusted for the pinned size. A
    /// build-time `Fit` falls back to the live window: either the game
    /// really is fit-to-parent, or the plugin was added before
    /// `WindowPlugin` and there was no window to read at build.
    pub fn at_startup(self, live_window: Option<&Window>) -> Self {
        match self {
            Self::Pinned { .. } => self,
            Self::Fit => live_window.map(Self::from_window).unwrap_or(Self::Fit),
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

    /// The pinned size as a reduced ratio, e.g. `"4:3"`. `None` for `Fit`
    /// (no authored aspect) and for a zero dimension.
    pub fn aspect_label(self) -> Option<String> {
        let Self::Pinned { width, height } = self else {
            return None;
        };
        if width == 0 || height == 0 {
            return None;
        }
        let g = gcd(width, height);
        Some(format!("{}:{}", width / g, height / g))
    }
}

fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
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
    fn sanctioned_sizes_carry_their_aspect_label() {
        assert_eq!(
            CanvasPolicy::PINNED_4X3,
            CanvasPolicy::Pinned {
                width: 960,
                height: 720
            }
        );
        assert_eq!(
            CanvasPolicy::PINNED_1X1,
            CanvasPolicy::Pinned {
                width: 720,
                height: 720
            }
        );
        assert_eq!(
            CanvasPolicy::PINNED_3X4,
            CanvasPolicy::Pinned {
                width: 720,
                height: 960
            }
        );

        let label = |p: CanvasPolicy| p.aspect_label();
        assert_eq!(label(CanvasPolicy::PINNED_720P).as_deref(), Some("16:9"));
        assert_eq!(label(CanvasPolicy::PINNED_4X3).as_deref(), Some("4:3"));
        assert_eq!(label(CanvasPolicy::PINNED_1X1).as_deref(), Some("1:1"));
        assert_eq!(label(CanvasPolicy::PINNED_3X4).as_deref(), Some("3:4"));
        // Fit has no authored aspect; a zero dimension has none either.
        assert_eq!(label(CanvasPolicy::Fit), None);
        assert_eq!(
            label(CanvasPolicy::Pinned {
                width: 0,
                height: 720
            }),
            None
        );
    }

    /// winit can resize the window to the canvas's device-pixel box before
    /// Startup (960 CSS px at DPR 2 reads as 1920). The size recorded at
    /// plugin build is what the game asked for and must win.
    #[test]
    fn a_pinned_size_recorded_at_build_survives_a_resized_window() {
        let mut live = CanvasPolicy::PINNED_4X3.window("t");
        live.resolution.set_physical_resolution(1920, 1440);
        assert_eq!(
            CanvasPolicy::from_window(&live),
            CanvasPolicy::Pinned {
                width: 1920,
                height: 1440
            },
            "precondition: the live window no longer shows the configured size"
        );
        assert_eq!(
            CanvasPolicy::PINNED_4X3.at_startup(Some(&live)),
            CanvasPolicy::PINNED_4X3
        );
        assert_eq!(
            CanvasPolicy::PINNED_4X3.at_startup(None),
            CanvasPolicy::PINNED_4X3
        );
    }

    /// A build-time `Fit` means either a real fit-to-parent game or a plugin
    /// added before `WindowPlugin`; only then is the live window consulted.
    #[test]
    fn a_fit_policy_at_build_falls_back_to_the_live_window() {
        let pinned = CanvasPolicy::PINNED_3X4.window("t");
        assert_eq!(
            CanvasPolicy::Fit.at_startup(Some(&pinned)),
            CanvasPolicy::PINNED_3X4
        );
        let fit = CanvasPolicy::Fit.window("t");
        assert_eq!(CanvasPolicy::Fit.at_startup(Some(&fit)), CanvasPolicy::Fit);
        assert_eq!(CanvasPolicy::Fit.at_startup(None), CanvasPolicy::Fit);
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
    /// `Fit` and stays wrong until Startup. This documents that fallback: in
    /// this misordered case the Startup fallback reads the live window, which
    /// can already carry a device-pixel size, so games must add the plugin
    /// after `DefaultPlugins`.
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
