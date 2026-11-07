//! The pixel format this client asks the server for. Always little endian and true colour.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixelFormat {
    pub bits_per_pixel: u8,
    pub depth: u8,
    pub max: [u16; 3],
    pub shift: [u8; 3],
}

impl PixelFormat {
    /// 32 bpp, 8 bits per channel (macOS Screen Sharing's native layout). Memory order B, G, R, X.
    pub fn rgb32(swap_red_blue: bool) -> Self {
        Self::new(32, 24, [255, 255, 255], [16, 8, 0], swap_red_blue)
    }

    pub fn rgb565(swap_red_blue: bool) -> Self {
        Self::new(16, 16, [31, 63, 31], [11, 5, 0], swap_red_blue)
    }

    /// 8 bpp: 3 bits red, 3 bits green, 2 bits blue.
    pub fn bgr233(swap_red_blue: bool) -> Self {
        Self::new(8, 8, [7, 7, 3], [0, 3, 6], swap_red_blue)
    }

    fn new(bpp: u8, depth: u8, max: [u16; 3], shift: [u8; 3], swap: bool) -> Self {
        let mut f = Self {
            bits_per_pixel: bpp,
            depth,
            max,
            shift,
        };
        if swap {
            f.max.swap(0, 2);
            f.shift.swap(0, 2);
        }
        f
    }

    pub fn bytes(&self) -> usize {
        usize::from(self.bits_per_pixel / 8)
    }

    /// ZRLE "compressed pixel": 32 bpp true colour with all channels in the low 24 bits uses 3 bytes.
    pub fn cpixel_bytes(&self) -> usize {
        if self.bits_per_pixel == 32 && self.depth <= 24 {
            3
        } else {
            self.bytes()
        }
    }

    /// Body of a SetPixelFormat message (16 bytes, following the three padding bytes).
    pub fn wire(&self) -> [u8; 16] {
        let mut b = [0u8; 16];
        b[0] = self.bits_per_pixel;
        b[1] = self.depth;
        b[2] = 0; // little endian
        b[3] = 1; // true colour
        for i in 0..3 {
            b[4 + i * 2..6 + i * 2].copy_from_slice(&self.max[i].to_be_bytes());
            b[10 + i] = self.shift[i];
        }
        b
    }

    /// Convert a little-endian pixel value to opaque RGBA.
    pub fn to_rgba(&self, value: u32) -> [u8; 4] {
        let ch = |i: usize| {
            let max = u32::from(self.max[i]);
            let v = (value >> self.shift[i]) & max;
            (v * 255 / max) as u8
        };
        [ch(0), ch(1), ch(2), 255]
    }

    /// Read a pixel of `n` little-endian bytes from the start of `bytes`.
    pub fn read(&self, bytes: &[u8], n: usize) -> u32 {
        bytes[..n]
            .iter()
            .enumerate()
            .fold(0u32, |acc, (i, b)| acc | (u32::from(*b) << (8 * i)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb32_decodes_bgrx_memory_order() {
        let f = PixelFormat::rgb32(false);
        let v = f.read(&[0x30, 0x20, 0x10, 0x00], 4);
        assert_eq!(f.to_rgba(v), [0x10, 0x20, 0x30, 255]);
        let v = f.read(&[0x30, 0x20, 0x10], 3);
        assert_eq!(f.to_rgba(v), [0x10, 0x20, 0x30, 255]);
        let swapped = PixelFormat::rgb32(true);
        assert_eq!(swapped.to_rgba(v), [0x30, 0x20, 0x10, 255]);
    }

    #[test]
    fn narrow_formats_scale_to_full_range() {
        let f = PixelFormat::rgb565(false);
        assert_eq!(f.to_rgba(0xffff), [255, 255, 255, 255]);
        assert_eq!(f.to_rgba(0xf800), [255, 0, 0, 255]);
        assert_eq!(f.to_rgba(0x001f), [0, 0, 255, 255]);
        let g = PixelFormat::bgr233(false);
        assert_eq!(g.to_rgba(0xff), [255, 255, 255, 255]);
        assert_eq!(g.to_rgba(0xc0), [0, 0, 255, 255]);
        assert_eq!((f.bytes(), g.bytes(), f.cpixel_bytes()), (2, 1, 2));
        assert_eq!(PixelFormat::rgb32(false).cpixel_bytes(), 3);
    }

    #[test]
    fn wire_format_is_sixteen_bytes() {
        let w = PixelFormat::rgb32(false).wire();
        assert_eq!(&w[..4], &[32, 24, 0, 1]);
        assert_eq!(&w[4..10], &[0, 255, 0, 255, 0, 255]);
        assert_eq!(&w[10..13], &[16, 8, 0]);
    }
}
