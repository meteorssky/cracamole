//! In-terminal prompts ("Login as: ", "Password: ", ...), as the C `guac_terminal_prompt`.

use guac_terminal::RemoteCmd;
use tokio::sync::mpsc;

/// Terminal grid size: (cols, rows, width_px, height_px).
pub type TermSize = (u16, u16, u32, u32);

/// The channels a [`Prompter`] owns, plus the last known terminal size.
pub type PrompterParts = (
    mpsc::Sender<Vec<u8>>,
    mpsc::Receiver<RemoteCmd>,
    Option<TermSize>,
);

/// Reads the user's keystrokes from the terminal and writes prompt text to it.
pub struct Prompter {
    to_term: mpsc::Sender<Vec<u8>>,
    from_term: mpsc::Receiver<RemoteCmd>,
    /// Latest grid size announced by the terminal (cols, rows, width_px, height_px).
    pub size: Option<TermSize>,
}

impl Prompter {
    pub fn new(to_term: mpsc::Sender<Vec<u8>>, from_term: mpsc::Receiver<RemoteCmd>) -> Self {
        Self {
            to_term,
            from_term,
            size: None,
        }
    }

    /// Take the channels back once prompting is over.
    pub fn into_parts(self) -> PrompterParts {
        (self.to_term, self.from_term, self.size)
    }

    pub async fn show(&self, text: &str) -> bool {
        self.to_term
            .send(text.replace('\n', "\r\n").into_bytes())
            .await
            .is_ok()
    }

    /// Show `title` and read one line. With `echo` false nothing typed is displayed.
    /// Returns `None` if the terminal went away or the user pressed Ctrl+C.
    pub async fn prompt(&mut self, title: &str, echo: bool) -> Option<String> {
        if !self.show(title).await {
            return None;
        }
        let mut line: Vec<u8> = vec![];
        loop {
            match self.from_term.recv().await? {
                RemoteCmd::Resize {
                    cols,
                    rows,
                    width_px,
                    height_px,
                } => {
                    self.size = Some((cols, rows, width_px, height_px));
                }
                RemoteCmd::Data(d) => {
                    for b in d {
                        match b {
                            b'\r' | b'\n' => {
                                self.show("\n").await;
                                return Some(String::from_utf8_lossy(&line).into_owned());
                            }
                            0x03 => {
                                self.show("\n").await;
                                return None;
                            }
                            0x7f | 0x08 => {
                                // remove one UTF-8 character
                                while let Some(last) = line.pop() {
                                    if last & 0xC0 != 0x80 {
                                        break;
                                    }
                                }
                                if echo {
                                    let _ = self.to_term.send(b"\x08 \x08".to_vec()).await;
                                }
                            }
                            c if c >= 0x20 => {
                                line.push(c);
                                if echo {
                                    let _ = self.to_term.send(vec![c]).await;
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }
}
