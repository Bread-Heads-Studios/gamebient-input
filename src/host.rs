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
    /// Pause state changed (also in answer to a host `gx:set` pause).
    Paused(bool),
    /// Anything else. `data` must already be valid JSON.
    Custom { name: String, data: String },
    /// The sealed replay of the run that just ended (`GXR1` bytes). Posted
    /// base64-encoded as `{"event":"run","replay":"..."}`.
    Run(Vec<u8>),
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
    /// A server-issued run seed (32 bytes) for the next run.
    Seed([u8; 32]),
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
    /// Also maintain the fixed-tick input path (`TickInput`) for games whose
    /// sim runs in `FixedUpdate`. Off by default.
    pub tick_input: bool,
    /// No window, no web glue: for headless verifiers and tests.
    pub headless: bool,
}

impl Default for GxConfig {
    fn default() -> Self {
        Self {
            name: "Gamebient Game".into(),
            aspect: "16:9".into(),
            extra_host_origins: Vec::new(),
            tick_input: false,
            headless: false,
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
        HostEvent::Paused(p) => format!("\"event\":\"paused\",\"paused\":{p}"),
        HostEvent::Custom { name, data } => {
            format!(
                "\"event\":\"custom\",\"name\":\"{}\",\"data\":{data}",
                json_escape(name)
            )
        }
        HostEvent::Run(bytes) => {
            format!("\"event\":\"run\",\"replay\":\"{}\"", base64_encode(bytes))
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

/// Standard base64 with padding (RFC 4648), no dependency.
pub fn base64_encode(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// 64 hex chars → 32 bytes.
pub fn parse_seed_hex(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, pair) in s.as_bytes().chunks(2).enumerate() {
        let hi = (pair[0] as char).to_digit(16)?;
        let lo = (pair[1] as char).to_digit(16)?;
        out[i] = (hi * 16 + lo) as u8;
    }
    Some(out)
}

/// Decodes the short command strings the JS side queues:
/// `hello:1|0`, `pause`, `resume`, `mute:1|0`, `seed:<64 hex>`.
pub fn parse_command(s: &str) -> Option<HostCommand> {
    if let Some(hex) = s.strip_prefix("seed:") {
        return parse_seed_hex(hex).map(HostCommand::Seed);
    }
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
            encode_event(&HostEvent::Paused(true)),
            r#"{"type":"gx:event","v":1,"event":"paused","paused":true}"#
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

    #[test]
    fn base64_matches_rfc4648() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn encodes_run_event_as_base64() {
        assert_eq!(
            encode_event(&HostEvent::Run(b"foo".to_vec())),
            r#"{"type":"gx:event","v":1,"event":"run","replay":"Zm9v"}"#
        );
    }

    #[test]
    fn parses_seed_hex() {
        let hex = "00".repeat(31) + "ff";
        let seed = parse_seed_hex(&hex).unwrap();
        assert_eq!(seed[31], 0xff);
        assert_eq!(seed[0], 0);
        assert!(parse_seed_hex("abc").is_none());
        assert!(parse_seed_hex(&"zz".repeat(32)).is_none());
    }

    #[test]
    fn parses_seed_command() {
        let hex = "ab".repeat(32);
        assert_eq!(
            parse_command(&format!("seed:{hex}")),
            Some(HostCommand::Seed([0xab; 32]))
        );
        assert_eq!(parse_command("seed:nope"), None);
    }
}
