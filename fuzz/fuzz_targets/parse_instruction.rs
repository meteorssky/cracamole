#![no_main]
use guac_protocol::{Parser, MAX_ARGS, MAX_ELEMENT_LEN};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut parser = Parser::new();
    for chunk in data.chunks(7) {
        let failed = parser.feed(chunk).is_err();
        while let Some(instruction) = parser.next_instruction() {
            assert!(instruction.args.len() <= MAX_ARGS);
            assert!(instruction.opcode.chars().count() <= MAX_ELEMENT_LEN);
            let mut replay = Parser::new();
            replay.feed(&instruction.encode()).unwrap();
            assert_eq!(replay.next_instruction(), Some(instruction));
        }
        if failed {
            break;
        }
    }
});
