/// Axis-aligned pixel rectangle. A rectangle with zero width or height is empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    pub fn new(x: i32, y: i32, w: u32, h: u32) -> Self {
        Self { x, y, w, h }
    }

    pub fn is_empty(&self) -> bool {
        self.w == 0 || self.h == 0
    }

    /// Exclusive right edge, saturating (no overflow).
    pub fn right(&self) -> i64 {
        self.x as i64 + self.w as i64
    }

    /// Exclusive bottom edge, saturating (no overflow).
    pub fn bottom(&self) -> i64 {
        self.y as i64 + self.h as i64
    }

    /// Rectangles that merely touch along an edge do NOT intersect.
    pub fn intersects(&self, o: &Rect) -> bool {
        !self.is_empty()
            && !o.is_empty()
            && (self.x as i64) < o.right()
            && (o.x as i64) < self.right()
            && (self.y as i64) < o.bottom()
            && (o.y as i64) < self.bottom()
    }

    pub fn intersection(&self, o: &Rect) -> Rect {
        if !self.intersects(o) {
            return Rect::new(0, 0, 0, 0);
        }
        let x = self.x.max(o.x);
        let y = self.y.max(o.y);
        let r = self.right().min(o.right());
        let b = self.bottom().min(o.bottom());
        Rect::new(x, y, (r - x as i64) as u32, (b - y as i64) as u32)
    }

    /// Smallest rectangle containing both. Empty operands are ignored.
    pub fn union(&self, o: &Rect) -> Rect {
        if self.is_empty() {
            return *o;
        }
        if o.is_empty() {
            return *self;
        }
        let x = self.x.min(o.x);
        let y = self.y.min(o.y);
        let r = self.right().max(o.right());
        let b = self.bottom().max(o.bottom());
        Rect::new(
            x,
            y,
            (r - x as i64).min(u32::MAX as i64) as u32,
            (b - y as i64).min(u32::MAX as i64) as u32,
        )
    }

    /// Clip to `0,0,w,h`.
    pub fn clip_to(&self, w: u32, h: u32) -> Rect {
        self.intersection(&Rect::new(0, 0, w, h))
    }
}
