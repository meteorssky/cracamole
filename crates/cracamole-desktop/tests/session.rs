use cracamole_desktop::{display::Display, session::Session};
use cracamole_store::Connection;
use guac_protocol::Instruction;
use std::{collections::BTreeMap, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

#[tokio::test]
async fn telnet_session_renders_a_real_server_frame_and_closes() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let fixture = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        socket.write_all(b"Native desktop test\r\n").await.unwrap();
        let mut bytes = [0; 1024];
        while socket.read(&mut bytes).await.unwrap() != 0 {}
    });
    let connection = Connection {
        id: "test".into(),
        name: "test".into(),
        protocol: "telnet".into(),
        params: BTreeMap::from([
            ("hostname".into(), "127.0.0.1".into()),
            ("port".into(), port.to_string()),
        ]),
    };
    let mut session = Session::connect(&connection, 640, 480).await.unwrap();
    let mut display = Display::default();
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let instruction = session
                .read()
                .await
                .unwrap()
                .expect("session closed before frame");
            match instruction.opcode.as_str() {
                "size" | "img" | "blob" | "end" => {
                    display.apply(&instruction).unwrap();
                }
                "sync" => {
                    session
                        .send(Instruction::new("sync", instruction.args))
                        .await
                        .unwrap();
                    if display
                        .pixels()
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .any(|pixel| pixel[0] > 100 && pixel[1] > 100 && pixel[2] > 100)
                    {
                        break;
                    }
                }
                "error" | "disconnect" => panic!("session ended before frame: {instruction:?}"),
                _ => {}
            }
        }
    })
    .await
    .expect("timed out waiting for rendered terminal");
    drop(session);
    tokio::time::timeout(Duration::from_secs(8), fixture)
        .await
        .expect("遠端連線未在期限內關閉")
        .unwrap();
}
