use std::fmt;

/// One Guacamole instruction: an opcode and its arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instruction {
    pub opcode: String,
    pub args: Vec<String>,
}

impl Instruction {
    pub fn new<S: Into<String>, A: Into<String>>(
        opcode: S,
        args: impl IntoIterator<Item = A>,
    ) -> Self {
        Self {
            opcode: opcode.into(),
            args: args.into_iter().map(Into::into).collect(),
        }
    }

    /// Append the wire encoding to `out`. Lengths count Unicode characters.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        fn element(out: &mut Vec<u8>, s: &str) {
            out.extend_from_slice(s.chars().count().to_string().as_bytes());
            out.push(b'.');
            out.extend_from_slice(s.as_bytes());
        }
        element(out, &self.opcode);
        for a in &self.args {
            out.push(b',');
            element(out, a);
        }
        out.push(b';');
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut v = Vec::new();
        self.encode_into(&mut v);
        v
    }
}

impl fmt::Display for Instruction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&String::from_utf8_lossy(&self.encode()))
    }
}
