use crate::Rect;
use base64::Engine;
use guac_protocol::Instruction;

/// Maximum decoded bytes per `blob` instruction. Must be a multiple of 3 so each
/// chunk base64-encodes without padding: guacamole.js concatenates the chunks.
pub const MAX_BLOB_BYTES: usize = 4095;

/// Largest width or height of the display in pixels. A client-chosen size must not be able to make
/// the server allocate an arbitrarily large framebuffer (4096x4096 RGBA is 64 MiB).
pub const MAX_DIMENSION: u32 = 4096;

/// Composite mode OVER (0xE). The framebuffer is fully opaque, so this equals a replace, but
/// SRC (0xC) maps to canvas `copy`, which browsers apply to the whole layer (it clears
/// everything outside the drawn rect), wiping the rest of the terminal on every partial update.
const COMP_OVER: &str = "14";

/// A single opaque layer (layer 0) backed by an RGBA framebuffer.
///
/// Drawing marks pixels dirty; `flush` turns the dirty area into `size`/`img`/`blob`/`end`
/// instructions. Dirty tracking is per pixel row (x-span), merged into horizontal bands.
pub struct Display {
    w: u32,
    h: u32,
    fb: Vec<u8>,
    /// Per pixel row: dirty x-span `[x0, x1)`.
    spans: Vec<Option<(u32, u32)>>,
    size_dirty: bool,
    next_stream: u32,
}

impl Display {
    pub fn new(w: u32, h: u32) -> Self {
        let (w, h) = (w.clamp(1, MAX_DIMENSION), h.clamp(1, MAX_DIMENSION));
        let mut d = Self {
            w,
            h,
            fb: vec![0; (w as usize) * (h as usize) * 4],
            spans: vec![None; h as usize],
            size_dirty: true,
            next_stream: 0,
        };
        d.mark_dirty(Rect::new(0, 0, w, h));
        d
    }

    pub fn size(&self) -> (u32, u32) {
        (self.w, self.h)
    }

    pub fn framebuffer(&self) -> &[u8] {
        &self.fb
    }

    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.w || y >= self.h {
            return None;
        }
        let o = ((y * self.w + x) * 4) as usize;
        Some([self.fb[o], self.fb[o + 1], self.fb[o + 2], self.fb[o + 3]])
    }

    /// Resize, preserving overlapping content. Everything becomes dirty and a `size` is emitted.
    pub fn resize(&mut self, w: u32, h: u32) {
        let (w, h) = (w.clamp(1, MAX_DIMENSION), h.clamp(1, MAX_DIMENSION));
        if (w, h) == (self.w, self.h) {
            return;
        }
        let mut nf = vec![0u8; (w as usize) * (h as usize) * 4];
        for y in 0..h.min(self.h) {
            let n = (w.min(self.w) * 4) as usize;
            let so = (y * self.w * 4) as usize;
            let dof = (y * w * 4) as usize;
            nf[dof..dof + n].copy_from_slice(&self.fb[so..so + n]);
        }
        self.fb = nf;
        self.w = w;
        self.h = h;
        self.spans = vec![None; h as usize];
        self.size_dirty = true;
        self.mark_dirty(Rect::new(0, 0, w, h));
    }

    pub fn mark_dirty(&mut self, r: Rect) {
        let r = r.clip_to(self.w, self.h);
        if r.is_empty() {
            return;
        }
        let (x0, x1) = (r.x as u32, r.x as u32 + r.w);
        for y in r.y as u32..r.y as u32 + r.h {
            let s = &mut self.spans[y as usize];
            *s = Some(match *s {
                Some((a, b)) => (a.min(x0), b.max(x1)),
                None => (x0, x1),
            });
        }
    }

    /// Fill a rectangle with a solid colour (clipped).
    pub fn fill_rect(&mut self, r: Rect, rgba: [u8; 4]) {
        let r = r.clip_to(self.w, self.h);
        if r.is_empty() {
            return;
        }
        for y in r.y as u32..r.y as u32 + r.h {
            for x in r.x as u32..r.x as u32 + r.w {
                let o = ((y * self.w + x) * 4) as usize;
                self.fb[o..o + 4].copy_from_slice(&rgba);
            }
        }
        self.mark_dirty(r);
    }

    /// Set one pixel without marking dirty (callers mark the enclosing rect once).
    pub fn put_pixel_raw(&mut self, x: u32, y: u32, rgba: [u8; 4]) {
        if x < self.w && y < self.h {
            let o = ((y * self.w + x) * 4) as usize;
            self.fb[o..o + 4].copy_from_slice(&rgba);
        }
    }

    /// Copy a region within the framebuffer (handles overlap) and mark the destination dirty.
    pub fn copy_within(&mut self, src: Rect, dx: i32, dy: i32) {
        let src = src.clip_to(self.w, self.h);
        if src.is_empty() {
            return;
        }
        let mut tmp = Vec::with_capacity((src.w * src.h * 4) as usize);
        for y in src.y as u32..src.y as u32 + src.h {
            let o = ((y * self.w + src.x as u32) * 4) as usize;
            tmp.extend_from_slice(&self.fb[o..o + (src.w * 4) as usize]);
        }
        let dst = Rect::new(dx, dy, src.w, src.h);
        for row in 0..src.h as i32 {
            for col in 0..src.w as i32 {
                let (tx, ty) = (dx + col, dy + row);
                if tx >= 0 && ty >= 0 && (tx as u32) < self.w && (ty as u32) < self.h {
                    let so = ((row as u32 * src.w + col as u32) * 4) as usize;
                    let d = ((ty as u32 * self.w + tx as u32) * 4) as usize;
                    self.fb[d..d + 4].copy_from_slice(&tmp[so..so + 4]);
                }
            }
        }
        self.mark_dirty(dst);
    }

    pub fn is_dirty(&self) -> bool {
        self.size_dirty || self.spans.iter().any(Option::is_some)
    }

    /// Dirty bands: consecutive dirty pixel rows merged, x-range = union of their spans.
    pub fn dirty_bands(&self) -> Vec<Rect> {
        let mut out: Vec<Rect> = vec![];
        let mut cur: Option<(u32, u32, u32, u32)> = None; // y0, y1, x0, x1
        for (y, s) in self.spans.iter().enumerate() {
            let y = y as u32;
            match (s, cur) {
                (Some((a, b)), Some((y0, _, x0, x1))) => {
                    cur = Some((y0, y + 1, x0.min(*a), x1.max(*b)))
                }
                (Some((a, b)), None) => cur = Some((y, y + 1, *a, *b)),
                (None, Some((y0, y1, x0, x1))) => {
                    out.push(Rect::new(x0 as i32, y0 as i32, x1 - x0, y1 - y0));
                    cur = None;
                }
                (None, None) => {}
            }
        }
        if let Some((y0, y1, x0, x1)) = cur {
            out.push(Rect::new(x0 as i32, y0 as i32, x1 - x0, y1 - y0));
        }
        out
    }

    fn encode_png(&self, r: Rect) -> Vec<u8> {
        let mut data = Vec::with_capacity((r.w * r.h * 4) as usize);
        for y in r.y as u32..r.y as u32 + r.h {
            let o = ((y * self.w + r.x as u32) * 4) as usize;
            data.extend_from_slice(&self.fb[o..o + (r.w * 4) as usize]);
        }
        let mut out = vec![];
        {
            let mut enc = png::Encoder::new(&mut out, r.w, r.h);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            enc.set_compression(png::Compression::Fast);
            let mut w = enc.write_header().expect("png header");
            w.write_image_data(&data).expect("png data");
        }
        out
    }

    /// Emit instructions for everything dirty, then clear dirty state.
    /// Returns an empty vec when nothing changed.
    pub fn flush(&mut self) -> Vec<Instruction> {
        let mut out = vec![];
        if self.size_dirty {
            out.push(Instruction::new(
                "size",
                ["0".to_string(), self.w.to_string(), self.h.to_string()],
            ));
            self.size_dirty = false;
        }
        for band in self.dirty_bands() {
            let png = self.encode_png(band);
            let id = self.next_stream.to_string();
            self.next_stream = (self.next_stream + 1) % 0x7FFF_FFFF;
            out.push(Instruction::new(
                "img",
                [
                    id.clone(),
                    COMP_OVER.to_string(),
                    "0".to_string(),
                    "image/png".to_string(),
                    band.x.to_string(),
                    band.y.to_string(),
                ],
            ));
            for chunk in png.chunks(MAX_BLOB_BYTES) {
                out.push(Instruction::new(
                    "blob",
                    [
                        id.clone(),
                        base64::engine::general_purpose::STANDARD.encode(chunk),
                    ],
                ));
            }
            out.push(Instruction::new("end", [id]));
        }
        self.spans.fill(None);
        out
    }
}
