#![no_main]
use libfuzzer_sys::fuzz_target;
use proto_telnet::{
    codec::{Decoder, Event, MAX_SB},
    negotiation::Negotiator,
};

fuzz_target!(|data: &[u8]| {
    let mut decoder = Decoder::new();
    let mut negotiator = Negotiator::new("linux", Some("tester"), 80, 24);
    for chunk in data.chunks(7) {
        for event in decoder.decode(chunk) {
            if let Event::Sb(_, payload) = &event {
                assert!(payload.len() <= MAX_SB);
            }
            let _ = negotiator.handle(event);
        }
    }
});
