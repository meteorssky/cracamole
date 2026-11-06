/// Mouse button bitmask used by the `mouse` instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MouseMask(pub u32);

impl MouseMask {
    pub const LEFT: u32 = 1;
    pub const MIDDLE: u32 = 2;
    pub const RIGHT: u32 = 4;
    pub const SCROLL_UP: u32 = 8;
    pub const SCROLL_DOWN: u32 = 16;

    pub fn has(self, bit: u32) -> bool {
        self.0 & bit != 0
    }
}
