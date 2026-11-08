use cracamole_desktop::display::Display;
use guac_display::{Display as ServerDisplay, Rect};
use guac_protocol::Instruction;

#[test]
fn renders_server_frames_and_preserves_untouched_pixels() {
    let mut server = ServerDisplay::new(16, 12);
    let mut client = Display::default();
    server.fill_rect(Rect::new(0, 0, 16, 12), [12, 34, 56, 255]);
    for instruction in server.flush() {
        client.apply(&instruction).unwrap();
    }
    assert_eq!(client.dimensions(), (16, 12));
    assert_eq!(client.pixels(), server.framebuffer());
    server.fill_rect(Rect::new(3, 4, 5, 2), [200, 100, 50, 255]);
    for instruction in server.flush() {
        client.apply(&instruction).unwrap();
    }
    assert_eq!(client.pixels(), server.framebuffer());
}

#[test]
fn rejects_oversized_frames_and_bad_streams() {
    let mut client = Display::default();
    assert!(client
        .apply(&Instruction::new("size", ["0", "4097", "2"]))
        .is_err());
    assert_eq!(client.dimensions(), (0, 0));
    client
        .apply(&Instruction::new("size", ["0", "2", "2"]))
        .unwrap();
    assert!(client
        .apply(&Instruction::new("blob", ["missing", "AAAA"]))
        .is_err());
    client
        .apply(&Instruction::new(
            "img",
            ["1", "14", "0", "image/png", "0", "0"],
        ))
        .unwrap();
    assert!(client
        .apply(&Instruction::new(
            "img",
            ["1", "14", "0", "image/png", "0", "0"]
        ))
        .is_err());
    assert!(client
        .apply(&Instruction::new("blob", ["1", "!!!"]))
        .is_err());
    assert!(client.apply(&Instruction::new("end", ["1"])).is_err());
    assert_eq!(client.pixels(), &[0; 16]);
}
