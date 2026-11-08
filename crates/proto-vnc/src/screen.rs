//! The remote framebuffer: validate untrusted desktop sizes before allocation and share a
//! process-wide render budget between sessions.

use guac_display::Display;
use guac_session::{SessionError, Status};
use std::sync::{Arc, OnceLock};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub const MAX_DIMENSION: u32 = guac_display::MAX_DIMENSION;
const RENDER_BUDGET_KIB: usize = 256 * 1024;
// Guacamole framebuffer (RGBA) plus PNG staging and transient encode buffers.
const BYTES_PER_PIXEL: usize = 8;

fn allocation_kib(width: u32, height: u32) -> Result<u32, SessionError> {
    if width == 0 || height == 0 || width.max(height) > MAX_DIMENSION {
        return Err(SessionError::plugin(
            Status::UpstreamError,
            format!(
                "The remote desktop is {width}x{height}, which exceeds the supported {MAX_DIMENSION}x{MAX_DIMENSION}. \
                 Lower the display resolution on the remote computer."
            ),
        ));
    }
    let bytes = (width as usize) * (height as usize) * BYTES_PER_PIXEL;
    Ok(bytes.div_ceil(1024) as u32)
}

pub struct Screen {
    pub display: Display,
    permit: Option<OwnedSemaphorePermit>,
}

fn budget() -> &'static Arc<Semaphore> {
    static BUDGET: OnceLock<Arc<Semaphore>> = OnceLock::new();
    BUDGET.get_or_init(|| Arc::new(Semaphore::new(RENDER_BUDGET_KIB)))
}

fn acquire(width: u32, height: u32) -> Result<OwnedSemaphorePermit, SessionError> {
    let kib = allocation_kib(width, height)?;
    budget().clone().try_acquire_many_owned(kib).map_err(|_| {
        SessionError::plugin(Status::ServerBusy, "VNC render memory budget exhausted.")
    })
}

impl Screen {
    pub fn new(width: u32, height: u32) -> Result<Self, SessionError> {
        let permit = acquire(width, height)?;
        Ok(Self {
            display: Display::new(width, height),
            permit: Some(permit),
        })
    }

    pub fn size(&self) -> (u32, u32) {
        self.display.size()
    }

    /// Change the desktop size. The old allocation is released first, so a failed resize ends the session.
    pub fn resize(&mut self, width: u32, height: u32) -> Result<(), SessionError> {
        self.permit = None;
        self.permit = Some(acquire(width, height)?);
        self.display.resize(width, height);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malicious_desktop_sizes_are_rejected_before_allocation() {
        for size in [
            (0, 768),
            (1024, 0),
            (4097, 1),
            (1, 4097),
            (u32::MAX, u32::MAX),
        ] {
            assert!(allocation_kib(size.0, size.1).is_err());
        }
        assert_eq!(allocation_kib(4096, 4096).unwrap(), 128 * 1024);
        assert!(Screen::new(u32::MAX, u32::MAX).is_err());
    }

    #[test]
    fn budget_is_shared_and_released_on_drop() {
        let budget = Arc::new(Semaphore::new(RENDER_BUDGET_KIB));
        let amount = allocation_kib(4096, 4096).unwrap();
        let a = budget.clone().try_acquire_many_owned(amount).unwrap();
        let b = budget.clone().try_acquire_many_owned(amount).unwrap();
        assert!(budget.clone().try_acquire_many_owned(1).is_err());
        drop(a);
        assert!(budget.clone().try_acquire_many_owned(amount).is_ok());
        drop(b);
    }

    #[test]
    fn resize_validates_and_keeps_the_screen_usable_size() {
        let mut screen = Screen::new(64, 48).unwrap();
        screen.resize(128, 96).unwrap();
        assert_eq!(screen.size(), (128, 96));
        assert!(screen.resize(5000, 100).is_err());
    }
}
