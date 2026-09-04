//! Keyboard / gamepad / virtual sources → one `GameInput` per frame.

use bevy::prelude::*;

use crate::buttons::{Buttons, Edges, edges};

/// Analog stick deadzone: values with |v| below this read as 0.
pub const STICK_DEADZONE: f32 = 0.2;

/// Frame-coherent input snapshot, populated once per frame by
/// [`collect_input`] from the keyboard, every connected gamepad and the
/// virtual source (touch overlay, host relay, Gamepad API on wasm), using
/// the Gamebient canon bindings:
///
/// - Move: arrows + WASD, D-pad, left stick, host axes
/// - A (primary): Z, Space · gamepad South or North · pad A
/// - B (secondary): X, Shift · gamepad West or East · pad B
/// - Start: Enter · pad Start
/// - Select: gamepad Select · pad Select
/// - Pause: Escape · gamepad Start · pad Pause
/// - Confirm (menus): A, B or Start
///
/// Gameplay systems read THIS resource, never raw input, so bindings live
/// in exactly one place.
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq)]
pub struct GameInput {
    pub move_x: f32,
    pub move_y: f32,
    pub primary_just_pressed: bool,
    pub primary_held: bool,
    pub primary_just_released: bool,
    pub secondary_just_pressed: bool,
    pub secondary_held: bool,
    pub secondary_just_released: bool,
    pub confirm_just_pressed: bool,
    pub start_just_pressed: bool,
    pub select_just_pressed: bool,
    pub pause_just_pressed: bool,
    /// Any canon button had a press edge this frame (studio-logo skip).
    pub any_just_pressed: bool,
    /// The raw edge set, for games that need a button the fields above
    /// don't name.
    pub edges: Edges,
}

/// Digital + analog state written by sources outside Bevy's own input
/// plugins: the web glue (touch overlay, `gx:input`, Gamepad API) on wasm,
/// or a native host such as the kiosk launcher. `latched` accumulates every
/// bit pressed since the last frame and is cleared by [`collect_input`].
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq)]
pub struct VirtualInput {
    pub held: Buttons,
    pub latched: Buttons,
    pub axis: Vec2,
}

impl VirtualInput {
    /// Replaces the held set, latching any newly pressed bits.
    pub fn set_held(&mut self, held: Buttons) {
        self.latched |= held.difference(self.held);
        self.held = held;
    }
}

/// Previous frame's union, kept between runs of [`collect_input`].
#[derive(Resource, Debug, Default)]
pub struct InputFrame {
    prev: Buttons,
}

/// Canon keyboard mapping. `pressed` answers "is this key down" (or "was it
/// just pressed", for the latch).
pub fn map_keys(pressed: impl Fn(KeyCode) -> bool) -> Buttons {
    use bevy::input::keyboard::KeyCode::*;
    let mut b = Buttons::NONE;
    let any = |keys: &[KeyCode]| keys.iter().any(|&k| pressed(k));
    if any(&[ArrowUp, KeyW]) {
        b |= Buttons::UP;
    }
    if any(&[ArrowDown, KeyS]) {
        b |= Buttons::DOWN;
    }
    if any(&[ArrowLeft, KeyA]) {
        b |= Buttons::LEFT;
    }
    if any(&[ArrowRight, KeyD]) {
        b |= Buttons::RIGHT;
    }
    if any(&[KeyZ, Space]) {
        b |= Buttons::A;
    }
    if any(&[KeyX, ShiftLeft, ShiftRight]) {
        b |= Buttons::B;
    }
    if pressed(Enter) {
        b |= Buttons::START;
    }
    if pressed(Escape) {
        b |= Buttons::PAUSE;
    }
    b
}

/// Canon gamepad mapping. South/North are A and West/East are B because
/// two-button arcade encoders (DragonRise) wire B unpredictably.
pub fn map_gamepad(pressed: impl Fn(GamepadButton) -> bool) -> Buttons {
    use bevy::input::gamepad::GamepadButton::*;
    let mut b = Buttons::NONE;
    if pressed(DPadUp) {
        b |= Buttons::UP;
    }
    if pressed(DPadDown) {
        b |= Buttons::DOWN;
    }
    if pressed(DPadLeft) {
        b |= Buttons::LEFT;
    }
    if pressed(DPadRight) {
        b |= Buttons::RIGHT;
    }
    if pressed(South) || pressed(North) {
        b |= Buttons::A;
    }
    if pressed(West) || pressed(East) {
        b |= Buttons::B;
    }
    if pressed(Select) {
        b |= Buttons::SELECT;
    }
    if pressed(Start) {
        b |= Buttons::PAUSE;
    }
    b
}

/// Applies the stick deadzone per axis.
pub fn apply_deadzone(v: Vec2) -> Vec2 {
    let f = |x: f32| if x.abs() < STICK_DEADZONE { 0.0 } else { x };
    Vec2::new(f(v.x), f(v.y))
}

/// Builds the frame's `GameInput` from the union edges plus analog input.
pub fn derive(e: Edges, analog: Vec2) -> GameInput {
    let axis = |neg: Buttons, pos: Buttons| {
        (e.held.contains(pos) as i32 - e.held.contains(neg) as i32) as f32
    };
    let jp = e.just_pressed;
    GameInput {
        move_x: (axis(Buttons::LEFT, Buttons::RIGHT) + analog.x).clamp(-1.0, 1.0),
        move_y: (axis(Buttons::DOWN, Buttons::UP) + analog.y).clamp(-1.0, 1.0),
        primary_just_pressed: jp.contains(Buttons::A),
        primary_held: e.held.contains(Buttons::A),
        primary_just_released: e.just_released.contains(Buttons::A),
        secondary_just_pressed: jp.contains(Buttons::B),
        secondary_held: e.held.contains(Buttons::B),
        secondary_just_released: e.just_released.contains(Buttons::B),
        confirm_just_pressed: jp.contains(Buttons::A | Buttons::B | Buttons::START),
        start_just_pressed: jp.contains(Buttons::START),
        select_just_pressed: jp.contains(Buttons::SELECT),
        pause_just_pressed: jp.contains(Buttons::PAUSE),
        any_just_pressed: !jp.is_empty(),
        edges: e,
    }
}

/// Populates [`GameInput`] from keyboard + all gamepads + the virtual
/// source. Runs in `PreUpdate` after Bevy's input systems, unconditionally,
/// so no state transition can leave stale just-* flags.
pub fn collect_input(
    keyboard: Res<ButtonInput<KeyCode>>,
    gamepads: Query<&Gamepad>,
    mut virt: ResMut<VirtualInput>,
    mut frame: ResMut<InputFrame>,
    mut input: ResMut<GameInput>,
) {
    let mut held = map_keys(|k| keyboard.pressed(k));
    let mut latched = map_keys(|k| keyboard.just_pressed(k));
    let mut analog = Vec2::ZERO;

    for pad in &gamepads {
        held |= map_gamepad(|b| pad.pressed(b));
        latched |= map_gamepad(|b| pad.just_pressed(b));
        analog += apply_deadzone(pad.left_stick());
    }

    held |= virt.held;
    latched |= core::mem::take(&mut virt.latched);
    analog += virt.axis;

    let e = edges(frame.prev, held, latched);
    frame.prev = e.held;
    *input = derive(e, analog);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn keys(set: &[KeyCode]) -> Buttons {
        let s: HashSet<KeyCode> = set.iter().copied().collect();
        map_keys(|k| s.contains(&k))
    }

    fn pad(set: &[GamepadButton]) -> Buttons {
        let s: HashSet<GamepadButton> = set.iter().copied().collect();
        map_gamepad(|b| s.contains(&b))
    }

    #[test]
    fn keyboard_canon() {
        assert_eq!(keys(&[KeyCode::KeyZ]), Buttons::A);
        assert_eq!(keys(&[KeyCode::Space]), Buttons::A);
        assert_eq!(keys(&[KeyCode::KeyX]), Buttons::B);
        assert_eq!(keys(&[KeyCode::ShiftLeft]), Buttons::B);
        assert_eq!(keys(&[KeyCode::Enter]), Buttons::START);
        assert_eq!(keys(&[KeyCode::Escape]), Buttons::PAUSE);
        assert_eq!(
            keys(&[KeyCode::ArrowLeft, KeyCode::KeyW]),
            Buttons::LEFT | Buttons::UP
        );
        assert!(keys(&[KeyCode::KeyQ]).is_empty());
    }

    #[test]
    fn gamepad_canon_with_dragonrise_pairing() {
        use bevy::input::gamepad::GamepadButton::*;
        assert_eq!(pad(&[South]), Buttons::A);
        assert_eq!(pad(&[North]), Buttons::A);
        assert_eq!(pad(&[West]), Buttons::B);
        assert_eq!(pad(&[East]), Buttons::B);
        assert_eq!(pad(&[Start]), Buttons::PAUSE);
        assert_eq!(pad(&[Select]), Buttons::SELECT);
        assert_eq!(pad(&[DPadRight, DPadDown]), Buttons::RIGHT | Buttons::DOWN);
    }

    #[test]
    fn deadzone() {
        assert_eq!(apply_deadzone(Vec2::new(0.1, -0.19)), Vec2::ZERO);
        assert_eq!(apply_deadzone(Vec2::new(0.5, -1.0)), Vec2::new(0.5, -1.0));
    }

    #[test]
    fn derive_movement_merges_digital_and_analog_and_clamps() {
        let e = edges(Buttons::NONE, Buttons::RIGHT | Buttons::UP, Buttons::NONE);
        let g = derive(e, Vec2::new(0.6, -0.3));
        assert_eq!(g.move_x, 1.0);
        assert!((g.move_y - 0.7).abs() < 1e-6);
        let g = derive(
            edges(Buttons::NONE, Buttons::LEFT, Buttons::NONE),
            Vec2::new(-1.0, 0.0),
        );
        assert_eq!(g.move_x, -1.0);
    }

    #[test]
    fn derive_actions_and_confirm() {
        let e = edges(Buttons::NONE, Buttons::A, Buttons::NONE);
        let g = derive(e, Vec2::ZERO);
        assert!(
            g.primary_just_pressed
                && g.primary_held
                && g.confirm_just_pressed
                && g.any_just_pressed
        );
        assert!(!g.secondary_just_pressed && !g.pause_just_pressed);

        let g = derive(
            edges(Buttons::NONE, Buttons::START, Buttons::NONE),
            Vec2::ZERO,
        );
        assert!(g.start_just_pressed && g.confirm_just_pressed);

        let g = derive(
            edges(Buttons::NONE, Buttons::PAUSE, Buttons::NONE),
            Vec2::ZERO,
        );
        assert!(g.pause_just_pressed && !g.confirm_just_pressed);

        let g = derive(edges(Buttons::A, Buttons::NONE, Buttons::NONE), Vec2::ZERO);
        assert!(g.primary_just_released && !g.primary_held && !g.any_just_pressed);
    }

    #[test]
    fn virtual_set_held_latches_new_presses_only() {
        let mut v = VirtualInput::default();
        v.set_held(Buttons::A);
        assert_eq!(v.latched, Buttons::A);
        v.latched = Buttons::NONE;
        v.set_held(Buttons::A | Buttons::B);
        assert_eq!(v.latched, Buttons::B);
        v.set_held(Buttons::NONE);
        assert_eq!(v.latched, Buttons::B);
    }

    #[test]
    fn collect_input_runs_in_an_app_and_consumes_the_virtual_latch() {
        let mut app = App::new();
        app.init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<VirtualInput>()
            .init_resource::<InputFrame>()
            .init_resource::<GameInput>()
            .add_systems(Update, collect_input);

        // A sub-frame tap arrives from the virtual source.
        app.world_mut().resource_mut::<VirtualInput>().latched = Buttons::START;
        app.update();
        let g = *app.world().resource::<GameInput>();
        assert!(g.start_just_pressed && g.confirm_just_pressed);
        assert!(app.world().resource::<VirtualInput>().latched.is_empty());

        // Next frame: nothing held, no repeat.
        app.update();
        let g = *app.world().resource::<GameInput>();
        assert!(!g.start_just_pressed && !g.any_just_pressed);

        // Held virtual A across frames: one press edge, then held.
        app.world_mut()
            .resource_mut::<VirtualInput>()
            .set_held(Buttons::A);
        app.update();
        assert!(app.world().resource::<GameInput>().primary_just_pressed);
        app.update();
        let g = *app.world().resource::<GameInput>();
        assert!(g.primary_held && !g.primary_just_pressed);
    }
}
