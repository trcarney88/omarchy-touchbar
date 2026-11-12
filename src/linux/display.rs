//! DRM lease framebuffer policy and adapter.
//!
//! The adapter only accepts an already-created lease descriptor; it never opens a
//! DRM node.  Display ownership and lease revocation remain with the coordinator.

#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
use drm::buffer::Buffer as _;
#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};

pub const WIDTH: usize = 60;
pub const HEIGHT: usize = 2008;
pub const SOURCE_WIDTH: usize = HEIGHT;
pub const SOURCE_HEIGHT: usize = WIDTH;
pub const BUFFER_WIDTH: u32 = 64;
#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
fn full_frame_damage<T>(
    mut submit: impl FnMut(&[drm::control::ClipRect]) -> std::io::Result<T>,
) -> std::io::Result<()> {
    use std::io::Error;
    let clips = [drm::control::ClipRect::new(
        0,
        0,
        BUFFER_WIDTH as u16,
        HEIGHT as u16,
    )];
    match submit(&clips) {
        Ok(_) => Ok(()),
        Err(error) if error.raw_os_error() == Some(libc::ENOSYS) => Ok(()),
        Err(error) => Err(Error::new(
            error.kind(),
            format!("DRM framebuffer dirty notification failed: {error}"),
        )),
    }
}

#[cfg(all(test, target_os = "linux", feature = "linux-hardware"))]
mod damage_tests {
    #[test]
    fn submits_full_frame_and_only_ignores_enosys() {
        use std::io::Error;
        super::full_frame_damage(|clips| {
            assert_eq!(clips.len(), 1);
            assert_eq!(clips[0], drm::control::ClipRect::new(0, 0, 64, 2008));
            Ok(())
        })
        .unwrap();
        assert!(
            super::full_frame_damage::<()>(|_| Err(Error::from_raw_os_error(libc::ENOSYS))).is_ok()
        );
        let error = super::full_frame_damage::<()>(|_| Err(Error::from_raw_os_error(libc::EINVAL)))
            .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("dirty notification"));
        assert!(error.to_string().contains("Invalid argument"));
    }
}
pub fn validate_display(name: &str, width: u32, height: u32) -> Result<(), &'static str> {
    if name != "DSI-1" {
        return Err("Unexpected connector (expected DSI-1)");
    }
    if (width, height) != (WIDTH as u32, HEIGHT as u32) {
        return Err("Refusing a display other than DSI-1 at 60x2008");
    }
    Ok(())
}

/// Rotate tightly packed RGBA (2008x60) into little-endian XRGB8888.
/// Bytes beyond the 60 pixels of each row are intentionally left untouched.
pub fn rgba_to_rotated_xrgb(
    source: &[u8],
    pitch: usize,
    destination: &mut [u8],
) -> Result<(), &'static str> {
    let row = WIDTH.checked_mul(4).ok_or("Invalid pitch")?;
    let required = pitch.checked_mul(HEIGHT).ok_or("Invalid pitch")?;
    if source.len()
        != SOURCE_WIDTH
            .checked_mul(SOURCE_HEIGHT)
            .and_then(|n| n.checked_mul(4))
            .ok_or("Invalid frame")?
        || pitch < row
        || destination.len() < required
    {
        return Err("Invalid framebuffer dimensions or pitch");
    }
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let sx = y;
            let sy = SOURCE_HEIGHT - 1 - x;
            let si = (sy * SOURCE_WIDTH + sx) * 4;
            let di = y * pitch + x * 4;
            destination[di..di + 4].copy_from_slice(&[
                source[si + 2],
                source[si + 1],
                source[si],
                0,
            ]);
        }
    }
    Ok(())
}

/// DRM adapter over a caller-supplied lease FD. Never opens a card path.
#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
pub struct DrmDisplay {
    guard: Guard,
    crtc: drm::control::crtc::Handle,
    fb: drm::control::framebuffer::Handle,
    pitch: usize,
    active_property: drm::control::property::Handle,
    active: Option<bool>,
}

#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
struct Card(OwnedFd);
#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
impl AsFd for Card {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}
#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
impl drm::Device for Card {}
#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
impl drm::control::Device for Card {}

#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
struct Guard {
    card: Card,
    db: Option<drm::control::dumbbuffer::DumbBuffer>,
    fb: Option<drm::control::framebuffer::Handle>,
    blob: Option<u64>,
    active: Option<(drm::control::crtc::Handle, drm::control::property::Handle)>,
}
#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
impl Drop for Guard {
    fn drop(&mut self) {
        use drm::control::{atomic::AtomicModeReq, AtomicCommitFlags, Device as _};
        if let Some((crtc, prop)) = self.active {
            let mut req = AtomicModeReq::new();
            req.add_property(crtc, prop, drm::control::property::Value::Boolean(false));
            let _ = self
                .card
                .atomic_commit(AtomicCommitFlags::ALLOW_MODESET, req);
        }
        if let Some(blob) = self.blob.take() {
            let _ = self.card.destroy_property_blob(blob);
        }
        if let Some(fb) = self.fb.take() {
            let _ = self.card.destroy_framebuffer(fb);
        }
        if let Some(db) = self.db.take() {
            let _ = self.card.destroy_dumb_buffer(db);
        }
    }
}

#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
struct SinglePlane<'a>(&'a drm::control::dumbbuffer::DumbBuffer);
#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
impl drm::buffer::PlanarBuffer for SinglePlane<'_> {
    fn size(&self) -> (u32, u32) {
        self.0.size()
    }
    fn format(&self) -> drm::buffer::DrmFourcc {
        drm::buffer::DrmFourcc::Xrgb8888
    }
    fn modifier(&self) -> Option<drm::buffer::DrmModifier> {
        None
    }
    fn pitches(&self) -> [u32; 4] {
        [self.0.pitch(), 0, 0, 0]
    }
    fn handles(&self) -> [Option<drm::buffer::Handle>; 4] {
        [Some(self.0.handle()), None, None, None]
    }
    fn offsets(&self) -> [u32; 4] {
        [0; 4]
    }
}

#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
impl DrmDisplay {
    pub fn from_lease_fd(fd: OwnedFd) -> std::io::Result<Self> {
        use drm::control::{
            atomic::AtomicModeReq, connector, property, AtomicCommitFlags, Device as _,
        };
        use drm::ClientCapability;
        use drm::Device as _;
        use std::io::{Error, ErrorKind};
        let mut guard = Guard {
            card: Card(fd),
            db: None,
            fb: None,
            blob: None,
            active: None,
        };
        guard
            .card
            .set_client_capability(ClientCapability::UniversalPlanes, true)?;
        guard
            .card
            .set_client_capability(ClientCapability::Atomic, true)?;
        let resources = guard.card.resource_handles()?;
        let mut found = Vec::new();
        let mut connector_count = 0usize;
        for handle in resources.connectors() {
            let info = guard.card.get_connector(*handle, true)?;
            if info.interface() == connector::Interface::DSI && info.interface_id() == 1 {
                connector_count += 1;
                if let Some(mode) = info
                    .modes()
                    .iter()
                    .find(|m| m.size() == (60, 2008))
                    .cloned()
                {
                    found.push((*handle, info, mode));
                }
            }
        }
        if connector_count != 1 || found.len() != 1 {
            return Err(Error::new(
                ErrorKind::NotFound,
                "Expected exactly one DSI-1 connector with a 60x2008 mode",
            ));
        }
        let (connector, _info, mode) = found.pop().unwrap();
        let connector_props = guard
            .card
            .get_properties(connector)?
            .as_hashmap(&guard.card)?;
        let connector_crtc = connector_props
            .get("CRTC_ID")
            .ok_or_else(|| Error::new(ErrorKind::Unsupported, "Missing CRTC_ID"))?
            .handle();
        let mut pair = None;
        for enc in _info.encoders() {
            let encoder = guard.card.get_encoder(*enc)?;
            for crtc in resources.filter_crtcs(encoder.possible_crtcs()) {
                let crtc_props = guard.card.get_properties(crtc)?.as_hashmap(&guard.card)?;
                let Some(mode_prop) = crtc_props.get("MODE_ID").map(|p| p.handle()) else {
                    continue;
                };
                let Some(active_prop) = crtc_props.get("ACTIVE").map(|p| p.handle()) else {
                    continue;
                };
                for ph in guard.card.plane_handles()? {
                    let p = guard.card.get_plane(ph)?;
                    if resources.filter_crtcs(p.possible_crtcs()).contains(&crtc)
                        && p.formats()
                            .contains(&(drm::buffer::DrmFourcc::Xrgb8888 as u32))
                    {
                        pair = Some((crtc, ph, mode_prop, active_prop));
                        break;
                    }
                }
                if pair.is_some() {
                    break;
                }
            }
            if pair.is_some() {
                break;
            }
        }
        let (crtc, plane, mode_prop, active_prop) = pair.ok_or_else(|| {
            Error::new(
                ErrorKind::Unsupported,
                "Lease has no compatible CRTC and XRGB8888 plane",
            )
        })?;
        let plane_props = guard.card.get_properties(plane)?.as_hashmap(&guard.card)?;
        let prop = |name: &str| {
            plane_props.get(name).map(|p| p.handle()).ok_or_else(|| {
                Error::new(
                    ErrorKind::Unsupported,
                    format!("Missing plane property {name}"),
                )
            })
        };
        let p_fb = prop("FB_ID")?;
        let p_crtc = prop("CRTC_ID")?;
        let sx = prop("SRC_X")?;
        let sy = prop("SRC_Y")?;
        let sw = prop("SRC_W")?;
        let sh = prop("SRC_H")?;
        let cx = prop("CRTC_X")?;
        let cy = prop("CRTC_Y")?;
        let cw = prop("CRTC_W")?;
        let ch = prop("CRTC_H")?;
        let db = guard.card.create_dumb_buffer(
            (BUFFER_WIDTH, HEIGHT as u32),
            drm::buffer::DrmFourcc::Xrgb8888,
            32,
        )?;
        let pitch = db.pitch() as usize;
        guard.db = Some(db);
        let db = guard.db.as_ref().unwrap();
        if pitch < WIDTH * 4 || pitch.checked_mul(HEIGHT).is_none() {
            return Err(Error::new(ErrorKind::InvalidData, "Invalid DRM pitch/size"));
        }
        let fb = guard
            .card
            .add_planar_framebuffer(&SinglePlane(db), drm::control::FbCmd2Flags::empty())?;
        guard.fb = Some(fb);
        {
            let mut mapping = guard.card.map_dumb_buffer(guard.db.as_mut().unwrap())?;
            if mapping.as_mut().len() < pitch * HEIGHT {
                return Err(Error::new(
                    ErrorKind::InvalidData,
                    "DRM mapping is too small",
                ));
            }
            mapping.as_mut().fill(0);
        }
        let blob: u64 = guard.card.create_property_blob(&mode)?.into();
        guard.blob = Some(blob);
        let mut req = AtomicModeReq::new();
        req.add_property(connector, connector_crtc, property::Value::CRTC(Some(crtc)));
        req.add_property(crtc, mode_prop, property::Value::Blob(blob));
        req.add_property(crtc, active_prop, property::Value::Boolean(false));
        req.add_property(plane, p_fb, property::Value::Framebuffer(Some(fb)));
        req.add_property(plane, p_crtc, property::Value::CRTC(Some(crtc)));
        req.add_property(plane, sx, property::Value::UnsignedRange(0));
        req.add_property(plane, sy, property::Value::UnsignedRange(0));
        req.add_property(
            plane,
            sw,
            property::Value::UnsignedRange((WIDTH as u64) << 16),
        );
        req.add_property(
            plane,
            sh,
            property::Value::UnsignedRange((HEIGHT as u64) << 16),
        );
        req.add_property(plane, cx, property::Value::SignedRange(0));
        req.add_property(plane, cy, property::Value::SignedRange(0));
        req.add_property(plane, cw, property::Value::UnsignedRange(WIDTH as u64));
        req.add_property(plane, ch, property::Value::UnsignedRange(HEIGHT as u64));
        guard
            .card
            .atomic_commit(AtomicCommitFlags::ALLOW_MODESET, req)?;
        guard.active = Some((crtc, active_prop));
        let _ = guard.card.destroy_property_blob(blob);
        guard.blob = None;
        Ok(Self {
            guard,
            crtc,
            fb,
            pitch,
            active_property: active_prop,
            active: Some(false),
        })
    }

    pub fn present(&mut self, rgba: &[u8]) -> std::io::Result<()> {
        use drm::control::Device as _;
        let db = self
            .guard
            .db
            .as_mut()
            .ok_or_else(|| std::io::Error::other("DRM buffer has been destroyed"))?;
        let mut mapping = self.guard.card.map_dumb_buffer(db)?;
        rgba_to_rotated_xrgb(rgba, self.pitch, mapping.as_mut())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
        drop(mapping);
        full_frame_damage(|clips| self.guard.card.dirty_framebuffer(self.fb, clips))
    }

    pub fn set_power(&mut self, active: bool) -> std::io::Result<()> {
        if self.active == Some(active) {
            return Ok(());
        }
        use drm::control::{atomic::AtomicModeReq, property, AtomicCommitFlags, Device as _};
        let mut req = AtomicModeReq::new();
        req.add_property(
            self.crtc,
            self.active_property,
            property::Value::Boolean(active),
        );
        if let Err(error) = self
            .guard
            .card
            .atomic_commit(AtomicCommitFlags::ALLOW_MODESET, req)
        {
            self.active = None;
            return Err(std::io::Error::new(
                error.kind(),
                format!("DRM set_power({active}) atomic commit failed: {error}"),
            ));
        }
        self.active = Some(active);
        Ok(())
    }

    /// Always submit a fresh OFF commit, even when the cached state is OFF/unknown.
    pub fn fresh_power_off(&mut self) -> std::io::Result<()> {
        self.active = None;
        self.set_power(false)
    }

    /// Clear only this adapter's already-leased framebuffer; this does not prove power-off.
    pub fn blackout(&mut self) -> std::io::Result<()> {
        use drm::control::Device as _;
        let db = self
            .guard
            .db
            .as_mut()
            .ok_or_else(|| std::io::Error::other("DRM buffer has been destroyed"))?;
        let mut mapping = self.guard.card.map_dumb_buffer(db)?;
        let required = self
            .pitch
            .checked_mul(HEIGHT)
            .ok_or_else(|| std::io::Error::other("Invalid DRM pitch/size"))?;
        if mapping.as_mut().len() < required {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "DRM mapping is too small",
            ));
        }
        mapping.as_mut()[..required].fill(0);
        drop(mapping);
        full_frame_damage(|clips| self.guard.card.dirty_framebuffer(self.fb, clips))
    }
}
