//! 在獨立執行緒讀取可能由其他程序延遲提供的剪貼簿內容。
use std::{
    sync::{mpsc, Arc},
    time::{Duration, Instant},
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

pub struct PasteResult {
    pub id: u64,
    pub text: Result<Zeroizing<String>, &'static str>,
    pub permit: OwnedSemaphorePermit,
    _worker: OwnedSemaphorePermit,
}

pub struct PasteJob {
    pub id: u64,
    pub deadline: Instant,
    cancel: CancellationToken,
}

impl Drop for PasteJob {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

pub struct PasteReader {
    sender: mpsc::Sender<PasteResult>,
    receiver: mpsc::Receiver<PasteResult>,
    worker: Arc<Semaphore>,
}

impl PasteReader {
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            sender,
            receiver,
            worker: Arc::new(Semaphore::new(1)),
        }
    }

    pub fn start(&self, id: u64, permit: OwnedSemaphorePermit) -> Result<PasteJob, &'static str> {
        self.start_with(id, permit, crate::windows_clipboard::read_text)
    }

    fn start_with<F>(
        &self,
        id: u64,
        permit: OwnedSemaphorePermit,
        read: F,
    ) -> Result<PasteJob, &'static str>
    where
        F: FnOnce() -> Result<String, &'static str> + Send + 'static,
    {
        let worker = self
            .worker
            .clone()
            .try_acquire_owned()
            .map_err(|_| "剪貼簿讀取仍在進行，請稍候。")?;
        let cancel = CancellationToken::new();
        let task_cancel = cancel.clone();
        let sender = self.sender.clone();
        std::thread::Builder::new()
            .name("cracamole-clipboard".into())
            .spawn(move || {
                let text = read().map(Zeroizing::new);
                if !task_cancel.is_cancelled() {
                    let _ = sender.send(PasteResult {
                        id,
                        text,
                        permit,
                        _worker: worker,
                    });
                }
            })
            .map_err(|_| "無法啟動剪貼簿讀取。")?;
        Ok(PasteJob {
            id,
            deadline: Instant::now() + Duration::from_secs(10),
            cancel,
        })
    }

    pub fn try_recv(&self) -> Option<PasteResult> {
        self.receiver.try_recv().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delayed_read_is_bounded_and_cancelled_content_is_discarded() {
        let reader = PasteReader::new();
        let paste = Arc::new(Semaphore::new(2));
        let (started, started_rx) = mpsc::channel();
        let (release, release_rx) = mpsc::channel();
        let job = reader
            .start_with(1, paste.clone().try_acquire_owned().unwrap(), move || {
                started.send(()).unwrap();
                release_rx.recv().unwrap();
                Ok("敏感文字".into())
            })
            .unwrap();
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(reader
            .start_with(2, paste.clone().try_acquire_owned().unwrap(), || Ok(
                "other".into()
            ))
            .is_err());
        drop(job);
        release.send(()).unwrap();
        let limit = Instant::now() + Duration::from_secs(2);
        while (reader.worker.available_permits() == 0 || paste.available_permits() != 2)
            && Instant::now() < limit
        {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(reader.worker.available_permits(), 1);
        assert_eq!(paste.available_permits(), 2);
        assert!(reader.try_recv().is_none());
    }

    #[test]
    fn successful_read_holds_permits_until_delivery() {
        let reader = PasteReader::new();
        let paste = Arc::new(Semaphore::new(1));
        let _job = reader
            .start_with(7, paste.clone().try_acquire_owned().unwrap(), || {
                Ok("hello".into())
            })
            .unwrap();
        let result = reader
            .receiver
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
        assert_eq!(result.id, 7);
        assert_eq!(result.text.as_ref().unwrap().as_str(), "hello");
        assert_eq!(reader.worker.available_permits(), 0);
        assert_eq!(paste.available_permits(), 0);
        drop(result);
        assert_eq!(reader.worker.available_permits(), 1);
        assert_eq!(paste.available_permits(), 1);
    }
}
