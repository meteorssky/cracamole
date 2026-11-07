//! Convert only PTY/Telnet text. Guacamole, JSON, SSH authentication and clipboard stay Unicode.

use encoding_rs::{
    CoderResult, Decoder, EncoderResult, Encoding, BIG5, EUC_JP, EUC_KR, GB18030, GBK, SHIFT_JIS,
    UTF_8, WINDOWS_1251, WINDOWS_1252,
};
use guac_session::{SessionError, Status};

#[derive(Clone, Copy, Debug)]
pub struct TextEncoding(&'static Encoding);

impl TextEncoding {
    pub fn parse(label: Option<&str>) -> Result<Self, SessionError> {
        let label = label
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("UTF-8");
        let alias = match label.to_ascii_lowercase().as_str() {
            "cp950" => Some(BIG5),
            "cp936" => Some(GBK),
            "cp932" => Some(SHIFT_JIS),
            "cp949" => Some(EUC_KR),
            _ => None,
        };
        let encoding = alias.or_else(|| Encoding::for_label(label.as_bytes()));
        let allowed = [
            UTF_8,
            BIG5,
            GBK,
            GB18030,
            SHIFT_JIS,
            EUC_JP,
            EUC_KR,
            WINDOWS_1251,
            WINDOWS_1252,
        ];
        encoding
            .filter(|e| allowed.contains(e))
            .map(Self)
            .ok_or_else(|| {
                SessionError::plugin(
                    Status::ClientBadRequest,
                    "Unsupported terminal character encoding.",
                )
            })
    }

    pub fn name(self) -> &'static str {
        self.0.name()
    }

    pub fn decoder(self) -> TextDecoder {
        TextDecoder {
            inner: self.0.new_decoder_without_bom_handling(),
            name: self.name(),
            warned: false,
        }
    }

    /// Atomic strict conversion: never emit HTML numeric references or a partial input command.
    pub fn encode(self, text: &str) -> Result<Vec<u8>, SessionError> {
        if self.0 == UTF_8 {
            return Ok(text.as_bytes().to_vec());
        }
        let mut encoder = self.0.new_encoder();
        let size = encoder
            .max_buffer_length_from_utf8_without_replacement(text.len())
            .ok_or_else(|| {
                SessionError::plugin(Status::ClientOverrun, "Terminal input is too large.")
            })?;
        let mut output = vec![0; size];
        let (result, read, written) =
            encoder.encode_from_utf8_without_replacement(text, &mut output, true);
        if result != EncoderResult::InputEmpty || read != text.len() {
            // Input may contain credentials: neither the character nor the text belongs in logs.
            return Err(SessionError::plugin(
                Status::ClientBadType,
                "Input cannot be represented in the selected terminal encoding.",
            ));
        }
        output.truncate(written);
        Ok(output)
    }

    pub fn encode_input(self, bytes: &[u8], backspace: u8) -> Result<Vec<u8>, SessionError> {
        // Backspace is an explicit byte setting, not a Unicode character (255 is allowed).
        if bytes == [backspace] {
            return Ok(bytes.to_vec());
        }
        let text = std::str::from_utf8(bytes).map_err(|_| {
            SessionError::plugin(Status::ClientBadType, "Terminal input is not valid UTF-8.")
        })?;
        self.encode(text)
    }
}

pub struct TextDecoder {
    inner: Decoder,
    name: &'static str,
    warned: bool,
}

impl TextDecoder {
    /// Decoder state survives arbitrary TCP/SSH chunk boundaries; EOF flushes incomplete input.
    pub fn decode(&mut self, bytes: &[u8], last: bool) -> Result<Vec<u8>, SessionError> {
        let size = self
            .inner
            .max_utf8_buffer_length(bytes.len())
            .ok_or_else(|| {
                SessionError::plugin(Status::UpstreamError, "Terminal output is too large.")
            })?;
        let mut output = String::with_capacity(size);
        let (result, read, replaced) = self.inner.decode_to_string(bytes, &mut output, last);
        if result != CoderResult::InputEmpty || read != bytes.len() {
            return Err(SessionError::plugin(
                Status::UpstreamError,
                "Unable to decode terminal output.",
            ));
        }
        if replaced && !self.warned {
            tracing::warn!(
                encoding = self.name,
                "Invalid terminal text bytes replaced; check the connection encoding"
            );
            self.warned = true;
        }
        Ok(output.into_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_byte_boundary_preserves_multibyte_characters_and_escape_sequences() {
        for (name, text, wire) in [
            ("UTF-8", "中文😀", "中文😀".as_bytes().to_vec()),
            ("Big5", "中文", vec![0xa4, 0xa4, 0xa4, 0xe5]),
            ("GBK", "中文", vec![0xd6, 0xd0, 0xce, 0xc4]),
            ("Shift_JIS", "日本", vec![0x93, 0xfa, 0x96, 0x7b]),
            ("EUC-JP", "日本", vec![0xc6, 0xfc, 0xcb, 0xdc]),
            ("EUC-KR", "가나", vec![0xb0, 0xa1, 0xb3, 0xaa]),
        ] {
            let encoding = TextEncoding::parse(Some(name)).unwrap();
            assert_eq!(encoding.encode(text).unwrap(), wire, "{name}");
            for split in 0..=wire.len() {
                let mut decoder = encoding.decoder();
                let mut out = decoder.decode(&wire[..split], false).unwrap();
                out.extend(decoder.decode(&wire[split..], true).unwrap());
                assert_eq!(out, text.as_bytes(), "{name}: split {split}");
            }
            let mut decoder = encoding.decoder();
            let mut out = Vec::new();
            for byte in &wire {
                out.extend(decoder.decode(&[*byte], false).unwrap());
            }
            out.extend(decoder.decode(b"\x1b[31m\r\n", true).unwrap());
            assert_eq!(out, [text.as_bytes(), b"\x1b[31m\r\n"].concat());
        }
    }

    #[test]
    fn unsupported_encodings_fail_closed_and_aliases_are_normalised() {
        assert_eq!(TextEncoding::parse(None).unwrap().name(), "UTF-8");
        assert_eq!(TextEncoding::parse(Some("cp950")).unwrap().name(), "Big5");
        assert_eq!(
            TextEncoding::parse(Some("cp932")).unwrap().name(),
            "Shift_JIS"
        );
        for bad in ["UTF-16LE", "ISO-2022-JP", "unknown"] {
            assert!(TextEncoding::parse(Some(bad)).is_err());
        }
    }

    #[test]
    fn unrepresentable_input_is_rejected_atomically_and_truncated_output_is_flushed() {
        let big5 = TextEncoding::parse(Some("Big5")).unwrap();
        assert_eq!(
            big5.encode("prefix😀suffix").unwrap_err().status(),
            Status::ClientBadType
        );
        assert_eq!(big5.encode_input(&[255], 255).unwrap(), [255]);
        let mut decoder = big5.decoder();
        assert!(decoder.decode(&[0xa4], false).unwrap().is_empty());
        assert_eq!(decoder.decode(&[], true).unwrap(), "�".as_bytes());
        let full = TextEncoding::parse(Some("GB18030")).unwrap();
        let wire = full.encode("中文😀").unwrap();
        assert_eq!(
            full.decoder().decode(&wire, true).unwrap(),
            "中文😀".as_bytes()
        );
    }
}
