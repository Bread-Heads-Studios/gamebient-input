//! The ColecoVision GX host protocol as Bevy messages, plus the JSON encoding
//! the web glue posts. See `docs/host-protocol.md`.

use core::fmt::Write as _;
use core::marker::PhantomData;

use bevy::prelude::*;
use bevy::state::state::StateTransitionEvent;

/// Protocol version carried in every message as `v`.
pub const PROTOCOL_VERSION: u32 = 1;

/// Game → host. Written by games (and by [`StateEvents`]); the web glue
/// posts each one to the host that answered `gx:hello`.
#[derive(Message, Debug, Clone, PartialEq)]
pub enum HostEvent {
    /// The engine is running and input is live. Posted once by the plugin.
    Ready,
    /// A `States` transition; `state` is the entered variant's `Debug` name.
    State(String),
    /// A run began.
    Started,
    /// A run ended.
    GameOver,
    /// Score update (final or running).
    Score(u64),
    /// Anything else. `data` must already be valid JSON.
    Custom { name: String, data: String },
}

/// Host → game. Emitted by the web glue when a `gx:hello` / `gx:set`
/// message arrives; games act on the ones they care about.
#[derive(Message, Debug, Clone, PartialEq)]
pub enum HostCommand {
    /// A host introduced itself. `host_has_controls` means the host draws
    /// its own pad, so the crate hides the in-page overlay.
    Hello {
        host_has_controls: bool,
    },
    Pause,
    Resume,
    Mute(bool),
}

/// Static description of this game for the `gx:hello` handshake.
#[derive(Resource, Debug, Clone, PartialEq)]
pub struct GxConfig {
    /// Display name reported to the host.
    pub name: String,
    /// Aspect the canvas is authored for, e.g. `"16:9"`.
    pub aspect: String,
    /// Extra host origins (exact, e.g. `https://partner.example`) accepted
    /// on top of the built-in pattern (production site, Vercel previews,
    /// localhost).
    pub extra_host_origins: Vec<String>,
}

impl Default for GxConfig {
    fn default() -> Self {
        Self {
            name: "Gamebient Game".into(),
            aspect: "16:9".into(),
            extra_host_origins: Vec::new(),
        }
    }
}

/// Escapes a string for inclusion inside JSON double quotes.
pub fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out
}

/// Encodes a [`HostEvent`] as the `gx:event` wire message.
pub fn encode_event(e: &HostEvent) -> String {
    let body = match e {
        HostEvent::Ready => "\"event\":\"ready\"".to_string(),
        HostEvent::State(s) => format!("\"event\":\"state\",\"state\":\"{}\"", json_escape(s)),
        HostEvent::Started => "\"event\":\"started\"".to_string(),
        HostEvent::GameOver => "\"event\":\"gameover\"".to_string(),
        HostEvent::Score(n) => format!("\"event\":\"score\",\"score\":{n}"),
        HostEvent::Custom { name, data } => {
            format!(
                "\"event\":\"custom\",\"name\":\"{}\",\"data\":{data}",
                json_escape(name)
            )
        }
    };
    format!("{{\"type\":\"gx:event\",\"v\":{PROTOCOL_VERSION},{body}}}")
}

/// Encodes the game's `gx:hello`.
pub fn encode_hello(config: &GxConfig, has_touch_controls: bool) -> String {
    format!(
        "{{\"type\":\"gx:hello\",\"v\":{PROTOCOL_VERSION},\"name\":\"{}\",\"aspect\":\"{}\",\"hasTouchControls\":{has_touch_controls}}}",
        json_escape(&config.name),
        json_escape(&config.aspect)
    )
}

/// Registers the protocol messages and posts `Ready` at startup.
pub struct HostPlugin;

impl Plugin for HostPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<HostEvent>()
            .add_message::<HostCommand>()
            .add_systems(Startup, |mut w: MessageWriter<HostEvent>| {
                w.write(HostEvent::Ready);
            });
    }
}

/// Posts a `state` event on every transition of `S`, so hosts get
/// `Menu` / `Playing` / `GameOver` for free.
pub struct StateEvents<S: States>(PhantomData<S>);

impl<S: States> Default for StateEvents<S> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

impl<S: States> Plugin for StateEvents<S> {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PostUpdate,
            |mut transitions: MessageReader<StateTransitionEvent<S>>,
             mut out: MessageWriter<HostEvent>| {
                for t in transitions.read() {
                    if let Some(entered) = &t.entered
                        && t.exited.as_ref() != Some(entered)
                    {
                        out.write(HostEvent::State(format!("{entered:?}")));
                    }
                }
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_json_strings() {
        assert_eq!(json_escape("a\"b\\c\n"), "a\\\"b\\\\c\\n");
        assert_eq!(json_escape("\u{1}"), "\\u0001");
    }

    #[test]
    fn encodes_events() {
        assert_eq!(
            encode_event(&HostEvent::Ready),
            r#"{"type":"gx:event","v":1,"event":"ready"}"#
        );
        assert_eq!(
            encode_event(&HostEvent::State("Playing".into())),
            r#"{"type":"gx:event","v":1,"event":"state","state":"Playing"}"#
        );
        assert_eq!(
            encode_event(&HostEvent::Score(1500)),
            r#"{"type":"gx:event","v":1,"event":"score","score":1500}"#
        );
        assert_eq!(
            encode_event(&HostEvent::Custom {
                name: "lap".into(),
                data: "{\"n\":2}".into()
            }),
            r#"{"type":"gx:event","v":1,"event":"custom","name":"lap","data":{"n":2}}"#
        );
    }

    #[test]
    fn encodes_hello() {
        let cfg = GxConfig {
            name: "Pizza \"Rush\"".into(),
            ..Default::default()
        };
        assert_eq!(
            encode_hello(&cfg, true),
            r#"{"type":"gx:hello","v":1,"name":"Pizza \"Rush\"","aspect":"16:9","hasTouchControls":true}"#
        );
    }

    #[derive(States, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    enum S {
        #[default]
        Menu,
        Playing,
    }

    #[test]
    fn state_events_post_on_transition_and_ready_on_startup() {
        let mut app = App::new();
        app.add_plugins(bevy::state::app::StatesPlugin)
            .init_state::<S>()
            .add_plugins(HostPlugin)
            .add_plugins(StateEvents::<S>::default());

        fn drain(app: &mut App) -> Vec<HostEvent> {
            app.world_mut()
                .resource_mut::<Messages<HostEvent>>()
                .drain()
                .collect()
        }

        app.update();
        assert_eq!(
            drain(&mut app),
            vec![HostEvent::Ready, HostEvent::State("Menu".into())]
        );

        app.world_mut()
            .resource_mut::<NextState<S>>()
            .set(S::Playing);
        app.update();
        assert_eq!(drain(&mut app), vec![HostEvent::State("Playing".into())]);
    }
}
