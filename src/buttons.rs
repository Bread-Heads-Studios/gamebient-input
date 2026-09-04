//! The digital button bitmask every input source is normalised to, and the
//! per-frame edge logic computed on the union of those sources.

/// A set of canon buttons packed into a `u32`. The bit layout is part of the
/// host protocol (`gx:input.buttons`), so it must never be reordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Buttons(pub u32);

impl Buttons {
    pub const NONE: Buttons = Buttons(0);
    pub const UP: Buttons = Buttons(1 << 0);
    pub const DOWN: Buttons = Buttons(1 << 1);
    pub const LEFT: Buttons = Buttons(1 << 2);
    pub const RIGHT: Buttons = Buttons(1 << 3);
    /// Primary action (keyboard Z / Space, gamepad South or North, pad A).
    pub const A: Buttons = Buttons(1 << 4);
    /// Secondary action (keyboard X / Shift, gamepad West or East, pad B).
    pub const B: Buttons = Buttons(1 << 5);
    /// Menu confirm (keyboard Enter, pad Start).
    pub const START: Buttons = Buttons(1 << 6);
    /// Gamepad Select / pad Select.
    pub const SELECT: Buttons = Buttons(1 << 7);
    /// Pause (keyboard Escape, gamepad Start, pad Pause).
    pub const PAUSE: Buttons = Buttons(1 << 8);

    /// Every bit the protocol defines; anything above is ignored.
    pub const ALL: Buttons = Buttons((1 << 9) - 1);

    #[inline]
    pub const fn contains(self, other: Buttons) -> bool {
        self.0 & other.0 != 0
    }

    #[inline]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    #[inline]
    pub const fn union(self, other: Buttons) -> Buttons {
        Buttons(self.0 | other.0)
    }

    #[inline]
    pub const fn difference(self, other: Buttons) -> Buttons {
        Buttons(self.0 & !other.0)
    }

    /// Masks to the protocol-defined bits.
    #[inline]
    pub const fn sanitized(self) -> Buttons {
        Buttons(self.0 & Self::ALL.0)
    }
}

impl core::ops::BitOr for Buttons {
    type Output = Buttons;
    fn bitor(self, rhs: Buttons) -> Buttons {
        self.union(rhs)
    }
}

impl core::ops::BitOrAssign for Buttons {
    fn bitor_assign(&mut self, rhs: Buttons) {
        self.0 |= rhs.0;
    }
}

/// Per-frame edge state for the union of all sources.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Edges {
    pub held: Buttons,
    pub just_pressed: Buttons,
    pub just_released: Buttons,
}

/// Derives edges from the previous frame's held set, this frame's held set,
/// and the buttons any source saw pressed since the last poll (`latched`).
///
/// The latch is what makes a sub-frame tap from a touch button or a host
/// relay count as a press. Because edges are computed on the union, a
/// button is only "released" once no device holds it.
pub fn edges(prev: Buttons, current: Buttons, latched: Buttons) -> Edges {
    let current = current.sanitized();
    let latched = latched.sanitized();
    Edges {
        held: current,
        just_pressed: current.union(latched).difference(prev),
        just_released: prev.difference(current),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bit_layout_is_the_protocol_layout() {
        assert_eq!(Buttons::UP.0, 1);
        assert_eq!(Buttons::DOWN.0, 2);
        assert_eq!(Buttons::LEFT.0, 4);
        assert_eq!(Buttons::RIGHT.0, 8);
        assert_eq!(Buttons::A.0, 16);
        assert_eq!(Buttons::B.0, 32);
        assert_eq!(Buttons::START.0, 64);
        assert_eq!(Buttons::SELECT.0, 128);
        assert_eq!(Buttons::PAUSE.0, 256);
        assert_eq!(Buttons::ALL.0, 511);
    }

    #[test]
    fn press_hold_release_edges() {
        let e = edges(Buttons::NONE, Buttons::A, Buttons::NONE);
        assert_eq!(e.just_pressed, Buttons::A);
        assert_eq!(e.held, Buttons::A);
        assert!(e.just_released.is_empty());

        let e = edges(Buttons::A, Buttons::A, Buttons::NONE);
        assert!(e.just_pressed.is_empty());
        assert_eq!(e.held, Buttons::A);

        let e = edges(Buttons::A, Buttons::NONE, Buttons::NONE);
        assert!(e.just_pressed.is_empty());
        assert_eq!(e.just_released, Buttons::A);
        assert!(e.held.is_empty());
    }

    #[test]
    fn sub_frame_tap_counts_via_latch() {
        // Pressed and released between two polls: current is empty but the
        // source latched it.
        let e = edges(Buttons::NONE, Buttons::NONE, Buttons::B);
        assert_eq!(e.just_pressed, Buttons::B);
        assert!(e.held.is_empty());
        assert!(e.just_released.is_empty());
    }

    #[test]
    fn latch_does_not_repeat_a_button_already_held() {
        let e = edges(Buttons::A, Buttons::A, Buttons::A);
        assert!(e.just_pressed.is_empty());
    }

    #[test]
    fn release_only_when_no_source_holds_it() {
        // Keyboard released Z while the gamepad still holds South: the union
        // stays held, so no release edge.
        let e = edges(Buttons::A, Buttons::A, Buttons::NONE);
        assert!(e.just_released.is_empty());
    }

    #[test]
    fn unknown_bits_are_masked() {
        let e = edges(
            Buttons::NONE,
            Buttons(1 << 20 | Buttons::UP.0),
            Buttons(1 << 30),
        );
        assert_eq!(e.held, Buttons::UP);
        assert_eq!(e.just_pressed, Buttons::UP);
    }
}
