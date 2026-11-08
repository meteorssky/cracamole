//! 解碼伺服器輸出的主圖層更新，並限制遠端資料的記憶體消耗。
use base64::{engine::general_purpose::STANDARD, Engine};
use guac_protocol::Instruction;
use std::{collections::HashMap, io::Cursor};

const MAX_DIMENSION: usize = 4096;
const MAX_STREAMS: usize = 4;
const MAX_ENCODED_BYTES: usize = 64 * 1024 * 1024;

struct ImageStream {
    x: usize,
    y: usize,
    bytes: Vec<u8>,
}

/// 單一遠端畫面的 RGBA 像素資料；錯誤指令會被拒絕。
#[derive(Default)]
pub struct Display {
    width: usize,
    height: usize,
    pixels: Vec<u8>,
    streams: HashMap<String, ImageStream>,
    stream_bytes: usize,
}

impl Display {
    pub fn contains_stream(&self, id: &str) -> bool {
        self.streams.contains_key(id)
    }
    pub fn dimensions(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// 回傳畫面是否更新；呼叫端須自行處理 sync、剪貼簿及連線狀態。
    pub fn apply(&mut self, instruction: &Instruction) -> Result<bool, &'static str> {
        let args = &instruction.args;
        let number = |i: usize| -> Result<usize, &'static str> {
            args.get(i)
                .ok_or("missing argument")?
                .parse()
                .map_err(|_| "invalid number")
        };
        match instruction.opcode.as_str() {
            "size" => {
                if args.len() != 3 || number(0)? != 0 {
                    return Err("unsupported layer");
                }
                let (width, height) = (number(1)?, number(2)?);
                if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
                    return Err("invalid display dimensions");
                }
                let mut pixels = vec![0; width * height * 4];
                for y in 0..height.min(self.height) {
                    let len = width.min(self.width) * 4;
                    pixels[y * width * 4..y * width * 4 + len].copy_from_slice(
                        &self.pixels[y * self.width * 4..y * self.width * 4 + len],
                    );
                }
                self.width = width;
                self.height = height;
                self.pixels = pixels;
                Ok(true)
            }
            "img" => {
                if args.len() != 6 || args[1] != "14" || args[2] != "0" || args[3] != "image/png" {
                    return Err("unsupported image");
                }
                if self.streams.len() >= MAX_STREAMS || self.streams.contains_key(&args[0]) {
                    return Err("too many image streams");
                }
                self.streams.insert(
                    args[0].clone(),
                    ImageStream {
                        x: number(4)?,
                        y: number(5)?,
                        bytes: Vec::new(),
                    },
                );
                Ok(false)
            }
            "blob" => {
                if args.len() != 2 {
                    return Err("invalid blob");
                }
                let stream = self
                    .streams
                    .get_mut(&args[0])
                    .ok_or("unknown image stream")?;
                // Check the encoded upper bound before allocating the decoded bytes.
                if args[1].len()
                    > MAX_ENCODED_BYTES
                        .saturating_sub(self.stream_bytes)
                        .saturating_mul(4)
                        / 3
                        + 4
                {
                    return Err("image data limit exceeded");
                }
                let bytes = STANDARD
                    .decode(&args[1])
                    .map_err(|_| "invalid image data")?;
                if bytes.len() > MAX_ENCODED_BYTES.saturating_sub(self.stream_bytes) {
                    return Err("image data limit exceeded");
                }
                self.stream_bytes += bytes.len();
                stream.bytes.extend_from_slice(&bytes);
                Ok(false)
            }
            "end" => {
                if args.len() != 1 {
                    return Err("invalid end");
                }
                let stream = self
                    .streams
                    .remove(&args[0])
                    .ok_or("unknown image stream")?;
                self.stream_bytes -= stream.bytes.len();
                self.blit(stream)?;
                Ok(true)
            }
            _ => Err("unsupported display instruction"),
        }
    }

    fn blit(&mut self, stream: ImageStream) -> Result<(), &'static str> {
        let decoder = png::Decoder::new(Cursor::new(stream.bytes));
        let mut reader = decoder.read_info().map_err(|_| "invalid png")?;
        let info = reader.info();
        let (width, height) = (info.width as usize, info.height as usize);
        if width == 0
            || height == 0
            || width > MAX_DIMENSION
            || height > MAX_DIMENSION
            || stream.x.checked_add(width).is_none_or(|x| x > self.width)
            || stream.y.checked_add(height).is_none_or(|y| y > self.height)
            || info.color_type != png::ColorType::Rgba
            || info.bit_depth != png::BitDepth::Eight
        {
            return Err("invalid image dimensions or format");
        }
        let size = reader.output_buffer_size().ok_or("invalid png buffer")?;
        if size != width * height * 4 {
            return Err("invalid png buffer");
        }
        let mut rgba = vec![0; size];
        reader
            .next_frame(&mut rgba)
            .map_err(|_| "invalid png frame")?;
        for y in 0..height {
            for x in 0..width {
                let source = (y * width + x) * 4;
                let target = ((stream.y + y) * self.width + stream.x + x) * 4;
                let alpha = u32::from(rgba[source + 3]);
                let back_alpha = u32::from(self.pixels[target + 3]);
                let out_alpha = alpha * 255 + back_alpha * (255 - alpha);
                if out_alpha == 0 {
                    continue;
                }
                for c in 0..3 {
                    let value = u32::from(rgba[source + c]) * alpha * 255
                        + u32::from(self.pixels[target + c]) * back_alpha * (255 - alpha);
                    self.pixels[target + c] = (value / out_alpha) as u8;
                }
                self.pixels[target + 3] = (out_alpha / 255) as u8;
            }
        }
        Ok(())
    }
}
