//! Server-to-client RFB messages and the rectangle encodings Raw, CopyRect, Hextile and ZRLE,
//! plus the DesktopSize / ExtendedDesktopSize pseudo-encodings.
//!
//! Everything read here comes from an untrusted peer: sizes are checked before allocation and
//! rectangles must lie inside the framebuffer.

use crate::format::PixelFormat;
use crate::screen::Screen;
use flate2::{Decompress, FlushDecompress, Status as ZStatus};
use guac_display::Rect;
use guac_session::{SessionError, Status};
use tokio::io::{AsyncRead, AsyncReadExt};

pub const ENC_RAW: i32 = 0;
pub const ENC_COPY_RECT: i32 = 1;
pub const ENC_HEXTILE: i32 = 5;
pub const ENC_ZRLE: i32 = 16;
pub const ENC_DESKTOP_SIZE: i32 = -223;
pub const ENC_EXTENDED_DESKTOP_SIZE: i32 = -308;

/// Encodings requested from the server, most preferred first.
pub const REQUESTED_ENCODINGS: &[i32] = &[
    ENC_ZRLE,
    ENC_HEXTILE,
    ENC_COPY_RECT,
    ENC_RAW,
    ENC_EXTENDED_DESKTOP_SIZE,
    ENC_DESKTOP_SIZE,
];

/// Clipboard text longer than this is skipped instead of buffered.
pub const MAX_CUT_TEXT: usize = 1 << 20;
/// Largest compressed ZRLE payload accepted for one rectangle.
const MAX_ZRLE_COMPRESSED: usize = 64 << 20;
/// Worst case decoded size of one 64x64 ZRLE tile (plain RLE with 1-pixel runs, 4 bytes each).
const ZRLE_TILE_WORST: usize = 64 * 64 * 4 + 16;
const MAX_NAME: usize = 4096;

#[derive(Debug, PartialEq, Eq)]
pub enum Message {
    /// A framebuffer update was applied; `resized` when the desktop size changed.
    Update {
        resized: bool,
    },
    Bell,
    CutText(Option<Vec<u8>>),
    ColourMap,
}

fn lost(e: std::io::Error) -> SessionError {
    SessionError::plugin(
        Status::UpstreamUnavailable,
        format!("VNC connection lost: {e}"),
    )
}

pub fn bad(message: impl Into<String>) -> SessionError {
    SessionError::plugin(
        Status::UpstreamError,
        format!("VNC protocol error: {}", message.into()),
    )
}

async fn skip<R: AsyncRead + Unpin>(rd: &mut R, mut n: usize) -> Result<(), SessionError> {
    let mut buf = [0u8; 4096];
    while n > 0 {
        let k = n.min(buf.len());
        rd.read_exact(&mut buf[..k]).await.map_err(lost)?;
        n -= k;
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
struct Region {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

impl Region {
    fn rect(&self) -> Rect {
        Rect::new(self.x as i32, self.y as i32, self.w, self.h)
    }
}

pub struct Decoder {
    format: PixelFormat,
    zrle: Decompress,
}

impl Decoder {
    pub fn new(format: PixelFormat) -> Self {
        Self {
            format,
            zrle: Decompress::new(true),
        }
    }

    /// Read and apply one server message.
    pub async fn read_message<R: AsyncRead + Unpin>(
        &mut self,
        rd: &mut R,
        screen: &mut Screen,
    ) -> Result<Message, SessionError> {
        let kind = rd.read_u8().await.map_err(lost)?;
        match kind {
            0 => {
                let mut head = [0u8; 3];
                rd.read_exact(&mut head).await.map_err(lost)?;
                let count = u16::from_be_bytes([head[1], head[2]]);
                let mut resized = false;
                for _ in 0..count {
                    resized |= self.read_rect(rd, screen).await?;
                }
                Ok(Message::Update { resized })
            }
            1 => {
                let mut head = [0u8; 5];
                rd.read_exact(&mut head).await.map_err(lost)?;
                let n = usize::from(u16::from_be_bytes([head[3], head[4]]));
                skip(rd, n * 6).await?;
                Ok(Message::ColourMap)
            }
            2 => Ok(Message::Bell),
            3 => {
                let mut head = [0u8; 7];
                rd.read_exact(&mut head).await.map_err(lost)?;
                let len = u32::from_be_bytes([head[3], head[4], head[5], head[6]]) as usize;
                if len > MAX_CUT_TEXT {
                    skip(rd, len).await?;
                    return Ok(Message::CutText(None));
                }
                let mut data = vec![0u8; len];
                rd.read_exact(&mut data).await.map_err(lost)?;
                Ok(Message::CutText(Some(data)))
            }
            other => Err(bad(format!("unsupported server message type {other}"))),
        }
    }

    /// Returns `true` when the rectangle changed the desktop size.
    async fn read_rect<R: AsyncRead + Unpin>(
        &mut self,
        rd: &mut R,
        screen: &mut Screen,
    ) -> Result<bool, SessionError> {
        let mut head = [0u8; 12];
        rd.read_exact(&mut head).await.map_err(lost)?;
        let x = u32::from(u16::from_be_bytes([head[0], head[1]]));
        let y = u32::from(u16::from_be_bytes([head[2], head[3]]));
        let w = u32::from(u16::from_be_bytes([head[4], head[5]]));
        let h = u32::from(u16::from_be_bytes([head[6], head[7]]));
        let encoding = i32::from_be_bytes([head[8], head[9], head[10], head[11]]);

        match encoding {
            ENC_DESKTOP_SIZE => {
                screen.resize(w, h)?;
                return Ok(true);
            }
            ENC_EXTENDED_DESKTOP_SIZE => {
                let mut info = [0u8; 4];
                rd.read_exact(&mut info).await.map_err(lost)?;
                skip(rd, usize::from(info[0]) * 16).await?;
                // `y` is the status code: non-zero means a request was refused, not a new size.
                if y != 0 {
                    return Ok(false);
                }
                if (w, h) != screen.size() {
                    screen.resize(w, h)?;
                    return Ok(true);
                }
                return Ok(false);
            }
            _ => {}
        }

        let (sw, sh) = screen.size();
        if x.checked_add(w).is_none_or(|r| r > sw) || y.checked_add(h).is_none_or(|b| b > sh) {
            return Err(bad(format!(
                "rectangle {w}x{h}+{x}+{y} lies outside the {sw}x{sh} framebuffer"
            )));
        }
        let region = Region { x, y, w, h };
        match encoding {
            ENC_RAW => self.raw(rd, screen, region).await?,
            ENC_COPY_RECT => copy_rect(rd, screen, region).await?,
            ENC_HEXTILE => self.hextile(rd, screen, region).await?,
            ENC_ZRLE => self.zrle(rd, screen, region).await?,
            other => return Err(bad(format!("unsupported encoding {other}"))),
        }
        Ok(false)
    }

    async fn raw<R: AsyncRead + Unpin>(
        &mut self,
        rd: &mut R,
        screen: &mut Screen,
        r: Region,
    ) -> Result<(), SessionError> {
        let bpp = self.format.bytes();
        let mut row = vec![0u8; r.w as usize * bpp];
        for dy in 0..r.h {
            rd.read_exact(&mut row).await.map_err(lost)?;
            for (dx, px) in row.chunks_exact(bpp).enumerate() {
                let rgba = self.format.to_rgba(self.format.read(px, bpp));
                screen
                    .display
                    .put_pixel_raw(r.x + dx as u32, r.y + dy, rgba);
            }
        }
        screen.display.mark_dirty(r.rect());
        Ok(())
    }

    async fn hextile<R: AsyncRead + Unpin>(
        &mut self,
        rd: &mut R,
        screen: &mut Screen,
        r: Region,
    ) -> Result<(), SessionError> {
        let fmt = self.format;
        let bpp = fmt.bytes();
        let mut background = [0, 0, 0, 255];
        let mut foreground = [255, 255, 255, 255];
        let mut buf = Vec::new();
        for ty in (0..r.h).step_by(16) {
            for tx in (0..r.w).step_by(16) {
                let (tw, th) = ((r.w - tx).min(16), (r.h - ty).min(16));
                let (px, py) = (r.x + tx, r.y + ty);
                let sub = rd.read_u8().await.map_err(lost)?;
                if sub & 1 != 0 {
                    buf.resize((tw * th) as usize * bpp, 0);
                    rd.read_exact(&mut buf).await.map_err(lost)?;
                    for (i, p) in buf.chunks_exact(bpp).enumerate() {
                        let (dx, dy) = (i as u32 % tw, i as u32 / tw);
                        screen.display.put_pixel_raw(
                            px + dx,
                            py + dy,
                            fmt.to_rgba(fmt.read(p, bpp)),
                        );
                    }
                    screen
                        .display
                        .mark_dirty(Rect::new(px as i32, py as i32, tw, th));
                    continue;
                }
                let mut pixel = [0u8; 4];
                if sub & 2 != 0 {
                    rd.read_exact(&mut pixel[..bpp]).await.map_err(lost)?;
                    background = fmt.to_rgba(fmt.read(&pixel, bpp));
                }
                if sub & 4 != 0 {
                    rd.read_exact(&mut pixel[..bpp]).await.map_err(lost)?;
                    foreground = fmt.to_rgba(fmt.read(&pixel, bpp));
                }
                screen
                    .display
                    .fill_rect(Rect::new(px as i32, py as i32, tw, th), background);
                if sub & 8 == 0 {
                    continue;
                }
                let coloured = sub & 16 != 0;
                let count = rd.read_u8().await.map_err(lost)?;
                for _ in 0..count {
                    let colour = if coloured {
                        rd.read_exact(&mut pixel[..bpp]).await.map_err(lost)?;
                        fmt.to_rgba(fmt.read(&pixel, bpp))
                    } else {
                        foreground
                    };
                    let xy = rd.read_u8().await.map_err(lost)?;
                    let wh = rd.read_u8().await.map_err(lost)?;
                    let (sx, sy) = (u32::from(xy >> 4), u32::from(xy & 15));
                    let (sw, sh) = (u32::from(wh >> 4) + 1, u32::from(wh & 15) + 1);
                    if sx + sw > tw || sy + sh > th {
                        return Err(bad("Hextile subrectangle outside its tile"));
                    }
                    screen.display.fill_rect(
                        Rect::new((px + sx) as i32, (py + sy) as i32, sw, sh),
                        colour,
                    );
                }
            }
        }
        Ok(())
    }

    async fn zrle<R: AsyncRead + Unpin>(
        &mut self,
        rd: &mut R,
        screen: &mut Screen,
        r: Region,
    ) -> Result<(), SessionError> {
        let len = rd.read_u32().await.map_err(lost)? as usize;
        if len > MAX_ZRLE_COMPRESSED {
            return Err(bad("ZRLE payload too large"));
        }
        let mut compressed = Vec::new();
        (&mut *rd)
            .take(len as u64)
            .read_to_end(&mut compressed)
            .await
            .map_err(lost)?;
        if compressed.len() != len {
            return Err(lost(std::io::ErrorKind::UnexpectedEof.into()));
        }
        if r.w == 0 || r.h == 0 {
            return Ok(());
        }
        let tiles = (r.w.div_ceil(64) as usize) * (r.h.div_ceil(64) as usize);
        let cap = tiles.saturating_mul(ZRLE_TILE_WORST);
        let mut data = Vec::new();
        inflate(&mut self.zrle, &compressed, &mut data, cap)?;
        decode_zrle(&data, &self.format, screen, r)
    }
}

async fn copy_rect<R: AsyncRead + Unpin>(
    rd: &mut R,
    screen: &mut Screen,
    r: Region,
) -> Result<(), SessionError> {
    let mut b = [0u8; 4];
    rd.read_exact(&mut b).await.map_err(lost)?;
    let sx = u32::from(u16::from_be_bytes([b[0], b[1]]));
    let sy = u32::from(u16::from_be_bytes([b[2], b[3]]));
    let (sw, sh) = screen.size();
    if sx + r.w > sw || sy + r.h > sh {
        return Err(bad("CopyRect source outside the framebuffer"));
    }
    screen.display.copy_within(
        Rect::new(sx as i32, sy as i32, r.w, r.h),
        r.x as i32,
        r.y as i32,
    );
    Ok(())
}

/// Inflate `input` through the connection-wide zlib stream, refusing to produce more than `cap` bytes.
fn inflate(
    z: &mut Decompress,
    input: &[u8],
    out: &mut Vec<u8>,
    cap: usize,
) -> Result<(), SessionError> {
    let start = z.total_in();
    loop {
        let used = (z.total_in() - start) as usize;
        if used >= input.len() && out.len() < out.capacity() {
            return Ok(());
        }
        if out.len() == out.capacity() {
            if out.len() >= cap {
                return Err(bad(
                    "ZRLE data expands beyond the rectangle's possible size",
                ));
            }
            out.reserve_exact((cap - out.len()).min(256 * 1024));
        }
        let before = (z.total_in(), out.len());
        let status = z
            .decompress_vec(&input[used..], out, FlushDecompress::Sync)
            .map_err(|e| bad(format!("invalid ZRLE stream: {e}")))?;
        if matches!(status, ZStatus::StreamEnd) {
            return Err(bad("ZRLE stream ended unexpectedly"));
        }
        if (z.total_in(), out.len()) == before && out.len() < out.capacity() {
            return Ok(());
        }
    }
}

struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], SessionError> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|e| *e <= self.data.len())
            .ok_or_else(|| bad("truncated ZRLE data"))?;
        let s = &self.data[self.pos..end];
        self.pos = end;
        Ok(s)
    }
    fn byte(&mut self) -> Result<u8, SessionError> {
        Ok(self.take(1)?[0])
    }
}

fn decode_zrle(
    data: &[u8],
    fmt: &PixelFormat,
    screen: &mut Screen,
    r: Region,
) -> Result<(), SessionError> {
    let cp = fmt.cpixel_bytes();
    let mut cur = Cursor { data, pos: 0 };
    let pixel = |c: &mut Cursor<'_>| -> Result<[u8; 4], SessionError> {
        let bytes = c.take(cp)?;
        Ok(fmt.to_rgba(fmt.read(bytes, cp)))
    };
    let mut tile_pixels: Vec<[u8; 4]> = Vec::new();
    for ty in (0..r.h).step_by(64) {
        for tx in (0..r.w).step_by(64) {
            let (tw, th) = ((r.w - tx).min(64), (r.h - ty).min(64));
            let total = (tw * th) as usize;
            let (px, py) = (r.x + tx, r.y + ty);
            let sub = cur.byte()?;
            let rle = sub & 0x80 != 0;
            let palette_size = usize::from(sub & 0x7f);
            tile_pixels.clear();
            match (rle, palette_size) {
                (false, 0) => {
                    for _ in 0..total {
                        tile_pixels.push(pixel(&mut cur)?);
                    }
                }
                (false, 1) => {
                    let colour = pixel(&mut cur)?;
                    screen
                        .display
                        .fill_rect(Rect::new(px as i32, py as i32, tw, th), colour);
                    continue;
                }
                (false, 2..=16) => {
                    let mut palette = Vec::with_capacity(palette_size);
                    for _ in 0..palette_size {
                        palette.push(pixel(&mut cur)?);
                    }
                    let bits = match palette_size {
                        2 => 1,
                        3..=4 => 2,
                        _ => 4,
                    };
                    for _ in 0..th {
                        let row_bytes = (tw as usize * bits).div_ceil(8);
                        let row = cur.take(row_bytes)?;
                        for x in 0..tw as usize {
                            let bit = x * bits;
                            let index = usize::from(
                                (row[bit / 8] >> (8 - bits - bit % 8)) & ((1u8 << bits) - 1),
                            );
                            tile_pixels.push(
                                *palette
                                    .get(index)
                                    .ok_or_else(|| bad("ZRLE palette index out of range"))?,
                            );
                        }
                    }
                }
                (true, 0) => {
                    while tile_pixels.len() < total {
                        let colour = pixel(&mut cur)?;
                        let run = run_length(&mut cur)?;
                        push_run(&mut tile_pixels, colour, run, total)?;
                    }
                }
                (true, 2..=127) => {
                    let mut palette = Vec::with_capacity(palette_size);
                    for _ in 0..palette_size {
                        palette.push(pixel(&mut cur)?);
                    }
                    while tile_pixels.len() < total {
                        let index = cur.byte()?;
                        let colour = *palette
                            .get(usize::from(index & 0x7f))
                            .ok_or_else(|| bad("ZRLE palette index out of range"))?;
                        let run = if index & 0x80 != 0 {
                            run_length(&mut cur)?
                        } else {
                            1
                        };
                        push_run(&mut tile_pixels, colour, run, total)?;
                    }
                }
                _ => return Err(bad(format!("invalid ZRLE subencoding {sub}"))),
            }
            for (i, rgba) in tile_pixels.iter().enumerate() {
                let (dx, dy) = (i as u32 % tw, i as u32 / tw);
                screen.display.put_pixel_raw(px + dx, py + dy, *rgba);
            }
            screen
                .display
                .mark_dirty(Rect::new(px as i32, py as i32, tw, th));
        }
    }
    Ok(())
}

fn run_length(cur: &mut Cursor<'_>) -> Result<usize, SessionError> {
    let mut len = 1usize;
    loop {
        let b = cur.byte()?;
        len += usize::from(b);
        if b != 255 {
            return Ok(len);
        }
    }
}

fn push_run(
    pixels: &mut Vec<[u8; 4]>,
    colour: [u8; 4],
    run: usize,
    total: usize,
) -> Result<(), SessionError> {
    if pixels.len() + run > total {
        return Err(bad("ZRLE run overruns its tile"));
    }
    pixels.resize(pixels.len() + run, colour);
    Ok(())
}

/// Parse the ServerInit name field length safely (used by the handshake).
pub fn checked_name_len(len: u32) -> Result<usize, SessionError> {
    let len = len as usize;
    if len > MAX_NAME {
        return Err(bad("desktop name too long"));
    }
    Ok(len)
}
