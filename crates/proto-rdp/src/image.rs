//! Validate untrusted desktop sizes before allocation and share a process-wide render budget.

use guac_session::{SessionError, Status};
use ironrdp_graphics::image_processing::PixelFormat;
use ironrdp_session::image::DecodedImage;
use std::ops::{Deref, DerefMut};
use std::sync::{Arc, OnceLock};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub const MAX_DIMENSION: u32 = guac_display::MAX_DIMENSION;
const RENDER_BUDGET_KIB: usize = 512 * 1024;
// Decoded RGBA, Guacamole framebuffer, PNG staging, and transient resize/encode buffers.
const BYTES_PER_PIXEL: usize = 16;

fn allocation_kib(width: u16, height: u16) -> Result<u32, SessionError> {
    if width == 0 || height == 0 || u32::from(width).max(u32::from(height)) > MAX_DIMENSION {
        return Err(SessionError::plugin(
            Status::UpstreamError,
            "RDP server desktop size exceeds the supported limits.",
        ));
    }
    let bytes = usize::from(width)
        .checked_mul(usize::from(height))
        .and_then(|pixels| pixels.checked_mul(BYTES_PER_PIXEL))
        .ok_or_else(|| SessionError::plugin(Status::UpstreamError, "RDP desktop size overflow."))?;
    Ok(bytes.div_ceil(1024) as u32)
}

pub struct Image {
    decoded: DecodedImage,
    _budget: OwnedSemaphorePermit,
}

impl Image {
    pub fn new(width: u16, height: u16) -> Result<Self, SessionError> {
        static BUDGET: OnceLock<Arc<Semaphore>> = OnceLock::new();
        let kib = allocation_kib(width, height)?;
        let budget = BUDGET.get_or_init(|| Arc::new(Semaphore::new(RENDER_BUDGET_KIB)));
        let permit = budget.clone().try_acquire_many_owned(kib).map_err(|_| {
            SessionError::plugin(Status::ServerBusy, "RDP render memory budget exhausted.")
        })?;
        Ok(Self {
            decoded: DecodedImage::new(PixelFormat::RgbA32, width, height),
            _budget: permit,
        })
    }
}

impl Deref for Image {
    type Target = DecodedImage;
    fn deref(&self) -> &Self::Target {
        &self.decoded
    }
}
impl DerefMut for Image {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.decoded
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
            (u16::MAX, u16::MAX),
        ] {
            assert!(allocation_kib(size.0, size.1).is_err());
        }
        assert_eq!(allocation_kib(4096, 4096).unwrap(), 256 * 1024);
        assert!(Image::new(u16::MAX, u16::MAX).is_err());
    }

    #[test]
    fn budget_covers_concurrent_images_and_releases_on_drop() {
        let budget = Arc::new(Semaphore::new(RENDER_BUDGET_KIB));
        let amount = allocation_kib(4096, 4096).unwrap();
        let a = budget.clone().try_acquire_many_owned(amount).unwrap();
        let b = budget.clone().try_acquire_many_owned(amount).unwrap();
        assert!(budget.clone().try_acquire_many_owned(1).is_err());
        drop(a);
        assert!(budget.clone().try_acquire_many_owned(amount).is_ok());
        drop(b);
    }
}
