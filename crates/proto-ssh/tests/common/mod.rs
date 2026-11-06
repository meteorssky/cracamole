//! In-process SSH server (russh) used to exercise the client plugin.
#![allow(dead_code)]

use russh::keys::{Algorithm, PrivateKey, PublicKey};
use russh::server::{self, Auth, Msg, Server as _, Session, VerifiedPublicKey};
use russh::{Channel, ChannelId, MethodKind, MethodSet, Pty};
use std::borrow::Cow;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Default, Debug, Clone)]
pub struct Recorded {
    pub auth_methods_tried: Vec<String>,
    pub auth_user: Option<String>,
    pub pty: Option<(String, u32, u32)>,
    pub modes: Vec<(Pty, u32)>,
    pub env: Vec<(String, String)>,
    pub window_changes: Vec<(u32, u32)>,
    pub exec: Option<String>,
    pub shell: bool,
    pub received: Vec<u8>,
    pub kbd_answers: Vec<String>,
}

#[derive(Clone)]
pub struct Behaviour {
    pub password: Option<(String, String)>,
    pub authorized_keys: Vec<PublicKey>,
    /// Keyboard-interactive: prompt text, echo flag, expected answer.
    pub kbd: Option<Vec<(String, bool, String)>>,
    pub refuse_pty: bool,
    /// Greeting written after the shell starts.
    pub greeting: String,
    /// Which auth methods to advertise.
    pub methods: Vec<MethodKind>,
    pub allow_none: bool,
}

impl Default for Behaviour {
    fn default() -> Self {
        Self {
            password: Some(("alice".into(), "secret".into())),
            authorized_keys: vec![],
            kbd: None,
            refuse_pty: false,
            greeting: "Welcome\r\n$ ".into(),
            methods: vec![
                MethodKind::Password,
                MethodKind::PublicKey,
                MethodKind::KeyboardInteractive,
            ],
            allow_none: false,
        }
    }
}

pub struct TestServer {
    pub addr: SocketAddr,
    pub rec: Arc<Mutex<Recorded>>,
    pub host_key: PrivateKey,
    _task: tokio::task::JoinHandle<()>,
}

#[derive(Clone)]
struct Srv {
    beh: Behaviour,
    rec: Arc<Mutex<Recorded>>,
    kbd_step: usize,
}

impl server::Server for Srv {
    type Handler = Self;
    fn new_client(&mut self, _: Option<SocketAddr>) -> Self {
        self.clone()
    }
}

impl server::Handler for Srv {
    type Error = russh::Error;

    async fn auth_none(&mut self, _user: &str) -> Result<Auth, Self::Error> {
        self.rec
            .lock()
            .unwrap()
            .auth_methods_tried
            .push("none".into());
        if self.beh.allow_none {
            Ok(Auth::Accept)
        } else {
            Ok(Auth::reject())
        }
    }

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        let mut r = self.rec.lock().unwrap();
        r.auth_methods_tried.push("password".into());
        r.auth_user = Some(user.to_string());
        match &self.beh.password {
            Some((u, p)) if u == user && p == password => Ok(Auth::Accept),
            _ => Ok(Auth::reject()),
        }
    }

    async fn auth_publickey(
        &mut self,
        user: &str,
        key: VerifiedPublicKey<'_>,
    ) -> Result<Auth, Self::Error> {
        let mut r = self.rec.lock().unwrap();
        r.auth_methods_tried.push("publickey".into());
        r.auth_user = Some(user.to_string());
        let offered: &PublicKey = key.public_key;
        if self
            .beh
            .authorized_keys
            .iter()
            .any(|k| k.key_data() == offered.key_data())
        {
            Ok(Auth::Accept)
        } else {
            Ok(Auth::reject())
        }
    }

    async fn auth_keyboard_interactive<'a>(
        &'a mut self,
        user: &str,
        _submethods: &str,
        response: Option<server::Response<'a>>,
    ) -> Result<Auth, Self::Error> {
        let Some(prompts) = self.beh.kbd.clone() else {
            return Ok(Auth::reject());
        };
        if self.kbd_step == 0 {
            self.rec
                .lock()
                .unwrap()
                .auth_methods_tried
                .push("keyboard-interactive".into());
            self.rec.lock().unwrap().auth_user = Some(user.to_string());
        }
        match response {
            None => {
                self.kbd_step = 0;
                Ok(Auth::Partial {
                    name: Cow::Borrowed("test"),
                    instructions: Cow::Borrowed(""),
                    prompts: Cow::Owned(
                        prompts
                            .iter()
                            .map(|(p, e, _)| (Cow::Owned(p.clone()), *e))
                            .collect(),
                    ),
                })
            }
            Some(resp) => {
                let answers: Vec<String> = resp
                    .map(|b| String::from_utf8_lossy(&b).into_owned())
                    .collect();
                self.rec.lock().unwrap().kbd_answers = answers.clone();
                let ok = answers.len() == prompts.len()
                    && answers
                        .iter()
                        .zip(&prompts)
                        .all(|(a, (_, _, want))| a == want);
                Ok(if ok { Auth::Accept } else { Auth::reject() })
            }
        }
    }

    async fn channel_open_session(
        &mut self,
        _channel: Channel<Msg>,
        reply: server::ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        reply.accept().await;
        Ok(())
    }

    async fn pty_request(
        &mut self,
        channel: ChannelId,
        term: &str,
        col_width: u32,
        row_height: u32,
        _pw: u32,
        _ph: u32,
        modes: &[(Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if self.beh.refuse_pty {
            session.channel_failure(channel)?;
            return Ok(());
        }
        let mut r = self.rec.lock().unwrap();
        r.pty = Some((term.to_string(), col_width, row_height));
        r.modes = modes.to_vec();
        session.channel_success(channel)?;
        Ok(())
    }

    async fn env_request(
        &mut self,
        channel: ChannelId,
        name: &str,
        value: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.rec
            .lock()
            .unwrap()
            .env
            .push((name.to_string(), value.to_string()));
        session.channel_success(channel)?;
        Ok(())
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.rec.lock().unwrap().shell = true;
        session.channel_success(channel)?;
        session.data(channel, self.beh.greeting.clone().into_bytes())?;
        Ok(())
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let cmd = String::from_utf8_lossy(data).into_owned();
        self.rec.lock().unwrap().exec = Some(cmd.clone());
        session.channel_success(channel)?;
        session.data(channel, format!("ran: {cmd}\r\n").into_bytes())?;
        session.exit_status_request(channel, 0)?;
        session.eof(channel)?;
        session.close(channel)?;
        Ok(())
    }

    async fn window_change_request(
        &mut self,
        _channel: ChannelId,
        col_width: u32,
        row_height: u32,
        _pw: u32,
        _ph: u32,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.rec
            .lock()
            .unwrap()
            .window_changes
            .push((col_width, row_height));
        Ok(())
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let done = {
            let mut r = self.rec.lock().unwrap();
            r.received.extend_from_slice(data);
            r.received.ends_with(b"exit\r")
        };
        // echo like a tty
        session.data(channel, data.to_vec())?;
        if done {
            session.data(channel, b"logout\r\n".to_vec())?;
            session.exit_status_request(channel, 0)?;
            session.eof(channel)?;
            session.close(channel)?;
        }
        Ok(())
    }
}

pub fn host_key() -> PrivateKey {
    PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).expect("generate host key")
}

pub async fn start_server(beh: Behaviour) -> TestServer {
    let host_key = host_key();
    let methods = MethodSet::from(&beh.methods[..]);
    let config = server::Config {
        keys: vec![host_key.clone()],
        methods,
        auth_rejection_time: Duration::from_millis(5),
        auth_rejection_time_initial: Some(Duration::from_millis(0)),
        inactivity_timeout: Some(Duration::from_secs(60)),
        ..Default::default()
    };
    let rec = Arc::new(Mutex::new(Recorded::default()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let mut srv = Srv {
        beh,
        rec: rec.clone(),
        kbd_step: 0,
    };
    let cfg = Arc::new(config);
    let task = tokio::spawn(async move {
        let _ = srv.run_on_socket(cfg, &listener).await;
    });
    TestServer {
        addr,
        rec,
        host_key,
        _task: task,
    }
}

/// Generate an OpenSSH key pair with the real `ssh-keygen`; `None` if the tool is unavailable.
pub fn keygen(kind: &str, bits: Option<&str>, passphrase: &str) -> Option<(String, PublicKey)> {
    let dir = std::env::temp_dir().join(format!(
        "cracamole-keygen-{}-{}-{}",
        std::process::id(),
        kind,
        rand_suffix()
    ));
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join("id");
    let mut cmd = std::process::Command::new("ssh-keygen");
    cmd.args(["-q", "-t", kind, "-N", passphrase, "-f"])
        .arg(&path);
    if let Some(b) = bits {
        cmd.args(["-b", b]);
    }
    let ok = cmd.status().ok()?.success();
    if !ok {
        return None;
    }
    let private = std::fs::read_to_string(&path).ok()?;
    let public = std::fs::read_to_string(path.with_extension("pub")).ok()?;
    let _ = std::fs::remove_dir_all(&dir);
    let pk = PublicKey::from_openssh(public.trim()).ok()?;
    Some((private, pk))
}

fn rand_suffix() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .subsec_nanos() as u64
        ^ (std::thread::current().id().as_u64_hack())
}

trait ThreadIdHack {
    fn as_u64_hack(&self) -> u64;
}
impl ThreadIdHack for std::thread::ThreadId {
    fn as_u64_hack(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.hash(&mut h);
        h.finish()
    }
}
