#![no_main]
use guac_protocol::{Instruction, Parser};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Keep each generated element inside the protocol's limit, including in large corpus files.
    let text = String::from_utf8_lossy(&data[..data.len().min(1024)]);
    let instruction = Instruction::new("key", [text.as_ref(), "中文😀,;."]);
    let wire = instruction.encode();
    for size in [1, 7, wire.len()] {
        let mut parser = Parser::new();
        for chunk in wire.chunks(size) {
            parser.feed(chunk).unwrap();
        }
        assert_eq!(parser.next_instruction(), Some(instruction.clone()));
        assert_eq!(parser.next_instruction(), None);
    }
});
