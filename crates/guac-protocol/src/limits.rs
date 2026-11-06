/// Maximum length (in characters) of one element. Matches C `GUAC_INSTRUCTION_MAX_LENGTH`.
pub const MAX_ELEMENT_LEN: usize = 8192;
/// Maximum number of elements (opcode + args) per instruction. Matches C `GUAC_INSTRUCTION_MAX_ELEMENTS`.
pub const MAX_ELEMENTS: usize = 128;
/// Maximum number of arguments (elements minus the opcode).
pub const MAX_ARGS: usize = MAX_ELEMENTS - 1;
