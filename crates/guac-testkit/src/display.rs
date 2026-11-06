use base64::Engine;
use guac_protocol::Instruction;
use std::collections::HashMap;

/// RGBA pixel.
pub type Pixel = [u8; 4];

#[derive(Default)]
struct Layer {
    w: usize,
    h: usize,
    px: Vec<Pixel>,
}

impl Layer {
    fn resize(&mut self, w: usize, h: usize) {
        let mut np = vec![[0, 0, 0, 0]; w * h];
        for y in 0..h.min(self.h) {
            for x in 0..w.min(self.w) {
                np[y * w + x] = self.px[y * self.w + x];
            }
        }
        self.w = w;
        self.h = h;
        self.px = np;
    }
}

/// Replays drawing instructions into pixel buffers (the verification equivalent of a Guacamole client).
/// Supports: size, rect+cfill, img/blob/end (raw RGBA `application/x-raw-rgba` test format and PNG),
/// copy.
#[derive(Default)]
pub struct DisplayReplay {
    layers: HashMap<i32, Layer>,
    streams: HashMap<String, (i32, i32, i32, String, Vec<u8>)>, // id -> (layer,x,y,mime,data)
    pending_rect: HashMap<i32, (i32, i32, i32, i32)>,
    pub unsupported: Vec<String>,
}

impl DisplayReplay {
    pub fn new() -> Self {
        Self::default()
    }

    fn layer(&mut self, i: i32) -> &mut Layer {
        self.layers.entry(i).or_default()
    }

    pub fn size(&self, layer: i32) -> (usize, usize) {
        self.layers
            .get(&layer)
            .map(|l| (l.w, l.h))
            .unwrap_or((0, 0))
    }

    pub fn pixel(&self, layer: i32, x: usize, y: usize) -> Option<Pixel> {
        let l = self.layers.get(&layer)?;
        if x < l.w && y < l.h {
            Some(l.px[y * l.w + x])
        } else {
            None
        }
    }

    pub fn apply(&mut self, ins: &Instruction) {
        let n = |i: usize| {
            ins.args
                .get(i)
                .and_then(|s| s.parse::<i32>().ok())
                .unwrap_or(0)
        };
        match ins.opcode.as_str() {
            "size" => {
                let (l, w, h) = (n(0), n(1).max(0) as usize, n(2).max(0) as usize);
                self.layer(l).resize(w, h);
            }
            "rect" => {
                self.pending_rect.insert(n(0), (n(1), n(2), n(3), n(4)));
            }
            "cfill" => {
                // cfill,mask,layer,r,g,b,a
                let layer = n(1);
                let color = [n(2) as u8, n(3) as u8, n(4) as u8, n(5) as u8];
                if let Some((x, y, w, h)) = self.pending_rect.remove(&layer) {
                    let l = self.layer(layer);
                    for yy in y.max(0)..(y + h).min(l.h as i32) {
                        for xx in x.max(0)..(x + w).min(l.w as i32) {
                            l.px[yy as usize * l.w + xx as usize] = color;
                        }
                    }
                }
            }
            "img" => {
                // img,stream,mask,layer,mimetype,x,y
                let id = ins.args[0].clone();
                self.streams
                    .insert(id, (n(2), n(4), n(5), ins.args[3].clone(), vec![]));
            }
            "blob" => {
                if let Some(s) = self.streams.get_mut(&ins.args[0]) {
                    if let Ok(b) = base64::engine::general_purpose::STANDARD.decode(&ins.args[1]) {
                        s.4.extend_from_slice(&b);
                    }
                }
            }
            "end" => {
                if let Some((layer, x, y, mime, data)) = self.streams.remove(&ins.args[0]) {
                    self.blit(layer, x, y, &mime, &data);
                }
            }
            "copy" => {
                // copy,srclayer,sx,sy,w,h,mask,dstlayer,dx,dy
                let (sl, sx, sy, w, h, dl, dx, dy) =
                    (n(0), n(1), n(2), n(3), n(4), n(6), n(7), n(8));
                let mut buf = vec![];
                if let Some(s) = self.layers.get(&sl) {
                    for yy in 0..h {
                        for xx in 0..w {
                            let (px, py) = (sx + xx, sy + yy);
                            if px >= 0 && py >= 0 && (px as usize) < s.w && (py as usize) < s.h {
                                buf.push((xx, yy, s.px[py as usize * s.w + px as usize]));
                            }
                        }
                    }
                }
                let d = self.layer(dl);
                for (xx, yy, p) in buf {
                    let (tx, ty) = (dx + xx, dy + yy);
                    if tx >= 0 && ty >= 0 && (tx as usize) < d.w && (ty as usize) < d.h {
                        d.px[ty as usize * d.w + tx as usize] = p;
                    }
                }
            }
            "sync" | "cursor" | "move" | "dispose" | "shade" | "nop" | "ack" | "ready" | "args" => {
            }
            other => self.unsupported.push(other.to_string()),
        }
    }

    fn blit(&mut self, layer: i32, x: i32, y: i32, mime: &str, data: &[u8]) {
        let (w, h, rgba): (usize, usize, Vec<u8>) = if mime == "image/x-raw-rgba" {
            // test-only format: u16 w, u16 h big-endian, then RGBA
            if data.len() < 4 {
                return;
            }
            let w = u16::from_be_bytes([data[0], data[1]]) as usize;
            let h = u16::from_be_bytes([data[2], data[3]]) as usize;
            (w, h, data[4..].to_vec())
        } else if mime == "image/png" {
            match decode_png(data) {
                Some(v) => v,
                None => {
                    self.unsupported.push("png-decode-failed".into());
                    return;
                }
            }
        } else {
            self.unsupported.push(format!("mime:{mime}"));
            return;
        };
        let l = self.layer(layer);
        for yy in 0..h {
            for xx in 0..w {
                let (tx, ty) = (x + xx as i32, y + yy as i32);
                if tx >= 0 && ty >= 0 && (tx as usize) < l.w && (ty as usize) < l.h {
                    let o = (yy * w + xx) * 4;
                    if o + 4 <= rgba.len() {
                        l.px[ty as usize * l.w + tx as usize] =
                            [rgba[o], rgba[o + 1], rgba[o + 2], rgba[o + 3]];
                    }
                }
            }
        }
    }
}

fn decode_png(data: &[u8]) -> Option<(usize, usize, Vec<u8>)> {
    let decoder = png::Decoder::new(std::io::Cursor::new(data));
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width as usize, info.height as usize);
    let bytes = &buf[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => bytes.to_vec(),
        png::ColorType::Rgb => bytes
            .chunks(3)
            .flat_map(|c| [c[0], c[1], c[2], 255])
            .collect(),
        png::ColorType::Grayscale => bytes.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::GrayscaleAlpha => bytes
            .chunks(2)
            .flat_map(|c| [c[0], c[0], c[0], c[1]])
            .collect(),
        png::ColorType::Indexed => return None,
    };
    Some((w, h, rgba))
}
