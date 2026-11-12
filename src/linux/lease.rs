//! DRM lease helpers and an explicitly-started Wayland lease adapter.

#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
mod adapter {
    use super::*;
    use std::{
        collections::HashMap,
        os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd},
        time::{Duration, Instant},
    };
    use wayland_client::{
        Connection, Dispatch, EventQueue, Proxy, QueueHandle,
        protocol::{wl_callback, wl_registry},
    };
    use wayland_protocols::wp::drm_lease::v1::client::{
        wp_drm_lease_connector_v1::{self, WpDrmLeaseConnectorV1},
        wp_drm_lease_device_v1::{self, WpDrmLeaseDeviceV1},
        wp_drm_lease_request_v1::{self, WpDrmLeaseRequestV1},
        wp_drm_lease_v1::{self, WpDrmLeaseV1},
    };

    #[derive(Default)]
    struct Offer {
        parent: u32,
        connector: Option<WpDrmLeaseConnectorV1>,
        name: Option<String>,
        id: Option<u32>,
        done: bool,
        withdrawn: bool,
    }
    #[derive(Default)]
    struct Device {
        proxy: Option<WpDrmLeaseDeviceV1>,
        done: bool,
        released: bool,
    }
    #[derive(Default)]
    struct ProtocolState {
        devices: HashMap<u32, Device>,
        offers: HashMap<wayland_client::backend::ObjectId, Offer>,
        registry_done: bool,
        fd: Option<OwnedFd>,
        lease: Option<WpDrmLeaseV1>,
        acquired: bool,
        terminal: Option<String>,
        requested: bool,
        registry: Option<wl_registry::WlRegistry>,
    }
    impl ProtocolState {
        fn fail(&mut self, why: impl Into<String>) {
            if self.terminal.is_none() {
                self.terminal = Some(why.into());
            }
            self.fd.take();
        }
        fn ready_to_request(&self) -> Result<Option<(u32, WpDrmLeaseConnectorV1)>, String> {
            if !self.registry_done {
                return Ok(None);
            }
            if self.devices.values().any(|d| !d.done && !d.released)
                || self.offers.values().any(|o| !o.withdrawn && !o.done)
            {
                return Ok(None);
            }
            let matches: Vec<_> = self
                .offers
                .iter()
                .filter(|(_, o)| {
                    !o.withdrawn
                        && o.done
                        && o.name.as_deref() == Some(CONNECTOR_NAME)
                        && o.id.is_some()
                })
                .collect();
            if matches.len() != 1 {
                return Err(if matches.is_empty() {
                    "No DSI-1 lease offer"
                } else {
                    "Ambiguous DSI-1 lease offers"
                }
                .into());
            }
            let (_, offer) = matches[0];
            Ok(Some((
                offer.parent,
                offer.connector.as_ref().unwrap().clone(),
            )))
        }
    }
    impl Dispatch<wl_registry::WlRegistry, ()> for ProtocolState {
        fn event(
            s: &mut Self,
            p: &wl_registry::WlRegistry,
            e: wl_registry::Event,
            _: &(),
            _: &Connection,
            q: &QueueHandle<Self>,
        ) {
            match e {
                wl_registry::Event::Global {
                    name,
                    interface,
                    version,
                } if interface == WpDrmLeaseDeviceV1::interface().name => {
                    if version < 1 {
                        s.fail("Unsupported DRM lease device version");
                        return;
                    }
                    let dev = p.bind::<WpDrmLeaseDeviceV1, u32, Self>(name, 1, q, name);
                    s.devices.insert(
                        name,
                        Device {
                            proxy: Some(dev),
                            ..Default::default()
                        },
                    );
                }
                wl_registry::Event::GlobalRemove { name } => {
                    if s.devices.contains_key(&name) {
                        s.fail("DRM lease device removed");
                    }
                }
                _ => {}
            }
        }
    }
    impl Dispatch<WpDrmLeaseDeviceV1, u32> for ProtocolState {
        wayland_client::event_created_child!(ProtocolState, WpDrmLeaseDeviceV1, [wp_drm_lease_device_v1::EVT_CONNECTOR_OPCODE => (WpDrmLeaseConnectorV1, ())]);
        fn event(
            s: &mut Self,
            p: &WpDrmLeaseDeviceV1,
            e: wp_drm_lease_device_v1::Event,
            id: &u32,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            match e {
                wp_drm_lease_device_v1::Event::Connector { id: child } => {
                    if let Some(device) = s.devices.get_mut(id) {
                        device.done = false;
                    }
                    s.offers.insert(
                        child.id(),
                        Offer {
                            parent: *id,
                            connector: Some(child),
                            ..Default::default()
                        },
                    );
                }
                wp_drm_lease_device_v1::Event::Done => {
                    if let Some(d) = s.devices.get_mut(id) {
                        d.done = true
                    }
                }
                wp_drm_lease_device_v1::Event::Released => {
                    if let Some(d) = s.devices.get_mut(id) {
                        d.released = true
                    }
                }
                wp_drm_lease_device_v1::Event::DrmFd { fd } => drop(fd),
                _ => {
                    let _ = p;
                }
            }
        }
    }
    impl Dispatch<WpDrmLeaseConnectorV1, ()> for ProtocolState {
        fn event(
            s: &mut Self,
            p: &WpDrmLeaseConnectorV1,
            e: wp_drm_lease_connector_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            let o = match s.offers.get_mut(&p.id()) {
                Some(o) => o,
                None => return,
            };
            match e {
                wp_drm_lease_connector_v1::Event::Name { name } => o.name = Some(name),
                wp_drm_lease_connector_v1::Event::ConnectorId { connector_id } => {
                    o.id = Some(connector_id)
                }
                wp_drm_lease_connector_v1::Event::Done => o.done = true,
                wp_drm_lease_connector_v1::Event::Withdrawn => o.withdrawn = true,
                _ => {}
            }
        }
    }
    impl Dispatch<WpDrmLeaseRequestV1, ()> for ProtocolState {
        fn event(
            _: &mut Self,
            _: &WpDrmLeaseRequestV1,
            _: wp_drm_lease_request_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }
    impl Dispatch<WpDrmLeaseV1, ()> for ProtocolState {
        fn event(
            s: &mut Self,
            p: &WpDrmLeaseV1,
            e: wp_drm_lease_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            match e {
                wp_drm_lease_v1::Event::LeaseFd { leased_fd } => {
                    if s.fd.is_none() && !s.acquired && s.terminal.is_none() {
                        s.fd = Some(leased_fd);
                        s.acquired = true
                    } else {
                        drop(leased_fd);
                        s.fail("Duplicate or late lease fd")
                    }
                }
                wp_drm_lease_v1::Event::Finished => {
                    s.fd.take();
                    s.acquired = false;
                    s.terminal = Some("DRM lease revoked".into());
                    p.destroy();
                }
                _ => {}
            }
        }
    }
    impl Dispatch<wl_callback::WlCallback, u32> for ProtocolState {
        fn event(
            s: &mut Self,
            _: &wl_callback::WlCallback,
            e: wl_callback::Event,
            _: &u32,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            match e {
                wl_callback::Event::Done { .. } => s.registry_done = true,
                _ => {}
            }
        }
    }

    pub struct Adapter {
        connection: Connection,
        queue: EventQueue<ProtocolState>,
        state: ProtocolState,
        deadline: Instant,
    }
    impl Adapter {
        pub fn start(timeout: Duration) -> Result<Self, String> {
            let connection = Connection::connect_to_env().map_err(|e| e.to_string())?;
            let mut queue = connection.new_event_queue();
            let qh = queue.handle();
            let mut state = ProtocolState::default();
            state.registry = Some(connection.display().get_registry(&qh, ()));
            connection.display().sync(&qh, 0);
            Ok(Self {
                connection,
                queue,
                state,
                deadline: Instant::now() + timeout,
            })
        }
        pub fn pump(&mut self, timeout: Duration) -> Result<(), String> {
            let until = if self.is_acquired() {
                Instant::now() + timeout
            } else {
                self.deadline.min(Instant::now() + timeout)
            };
            loop {
                self.queue.dispatch_pending(&mut self.state).map_err(|e| {
                    self.state.fail(e.to_string());
                    e.to_string()
                })?;
                if let Some(e) = self.state.terminal.clone() {
                    return Err(e);
                }
                match self.state.ready_to_request() {
                    Err(e) => {
                        self.state.fail(e.clone());
                        return Err(e);
                    }
                    Ok(Some((parent, connector))) if !self.state.requested => {
                        let d = self
                            .state
                            .devices
                            .get(&parent)
                            .and_then(|d| d.proxy.as_ref())
                            .ok_or("Lease device disappeared")?;
                        let qh = self.queue.handle();
                        let req = d.create_lease_request(&qh, ());
                        req.request_connector(&connector);
                        let lease = req.submit(&qh, ());
                        self.state.lease = Some(lease);
                        self.state.requested = true;
                    }
                    _ => {}
                }
                if self.state.fd.is_none() && Instant::now() >= self.deadline {
                    self.state.fail("Timed out waiting for DRM lease");
                    return Err("Timed out waiting for DRM lease".into());
                }
                if let Err(e) = self.connection.flush() {
                    if !matches!(e, wayland_client::backend::WaylandError::Io(ref io) if io.kind()==std::io::ErrorKind::WouldBlock)
                    {
                        self.state.fail(e.to_string());
                        return Err(e.to_string());
                    }
                }
                let guard = match self.connection.prepare_read() {
                    Some(g) => g,
                    None => {
                        if Instant::now() >= until {
                            return Ok(());
                        }
                        std::thread::yield_now();
                        continue;
                    }
                };
                let mut p = libc::pollfd {
                    fd: self.connection.as_fd().as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                };
                let ms = if self.is_acquired() {
                    0
                } else {
                    until
                        .saturating_duration_since(Instant::now())
                        .as_millis()
                        .min(i32::MAX as u128) as i32
                };
                let n = unsafe { libc::poll(&mut p, 1, ms) };
                if n < 0 {
                    let e = std::io::Error::last_os_error();
                    drop(guard);
                    if e.kind() == std::io::ErrorKind::Interrupted {
                        if Instant::now() >= until {
                            return Ok(());
                        }
                        continue;
                    }
                    self.state.fail(e.to_string());
                    return Err(e.to_string());
                }
                if p.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
                    drop(guard);
                    self.state.fail("Wayland connection failed");
                    return Err("Wayland connection failed".into());
                }
                if n > 0 {
                    if let Err(e) = guard.read() {
                        if !matches!(e, wayland_client::backend::WaylandError::Io(ref io) if io.kind()==std::io::ErrorKind::WouldBlock)
                        {
                            self.state.fail(e.to_string());
                            return Err(e.to_string());
                        }
                    }
                } else {
                    drop(guard)
                }
                if Instant::now() >= until && self.is_acquired() {
                    return Ok(());
                }
            }
        }
        pub fn fd(&self) -> Option<BorrowedFd<'_>> {
            self.state.fd.as_ref().map(AsFd::as_fd)
        }
        pub fn socket_fd(&self) -> BorrowedFd<'_> {
            self.connection.as_fd()
        }
        pub fn leased_fd(&self) -> Option<BorrowedFd<'_>> {
            self.fd()
        }
        pub fn is_acquired(&self) -> bool {
            self.state.acquired && self.state.fd.is_some()
        }
        pub fn status(&self) -> Result<(), &str> {
            self.state.terminal.as_deref().map_or(Ok(()), Err)
        }
    }
    impl Drop for Adapter {
        fn drop(&mut self) {
            if let Some(l) = self.state.lease.take() {
                l.destroy()
            }
            for d in self.state.devices.values() {
                if let Some(p) = &d.proxy {
                    if !d.released {
                        p.release()
                    }
                }
            }
            self.state.fd.take();
        }
    }
}
#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
pub use adapter::Adapter;

pub const CONNECTOR_NAME: &str = "DSI-1";
pub const PANEL_WIDTH: usize = 60;
pub const PANEL_HEIGHT: usize = 2008;
pub const IMAGE_WIDTH: usize = PANEL_HEIGHT;
pub const IMAGE_HEIGHT: usize = PANEL_WIDTH;

pub fn choose_connector<'a>(
    names: impl IntoIterator<Item = &'a str>,
) -> Result<&'a str, &'static str> {
    let mut matches = names.into_iter().filter(|name| *name == CONNECTOR_NAME);
    let found = matches.next().ok_or("No DSI-1 lease offer")?;
    if matches.next().is_some() {
        return Err("Ambiguous DSI-1 lease offers");
    }
    Ok(found)
}

pub fn validate_mode(width: u32, height: u32, _refresh_millihz: u32) -> Result<(), &'static str> {
    if (width, height) != (60, 2008) {
        return Err("Refusing a display other than 60x2008");
    }
    Ok(())
}

/// Rotate the legacy Qt +90-degree presentation into padded little-endian XRGB8888.
/// Source is tightly packed RGBA, dimensions 2008x60; destination pitch is bytes.
pub fn rgba_to_rotated_xrgb(
    source: &[u8],
    pitch: usize,
    destination: &mut [u8],
) -> Result<(), &'static str> {
    let row_bytes = PANEL_WIDTH.checked_mul(4).ok_or("Invalid pitch")?;
    if source.len() != IMAGE_WIDTH * IMAGE_HEIGHT * 4
        || pitch < row_bytes
        || destination.len() < pitch.checked_mul(PANEL_HEIGHT).ok_or("Invalid pitch")?
    {
        return Err("Invalid framebuffer dimensions or pitch");
    }
    for y in 0..PANEL_HEIGHT {
        let row = &mut destination[y * pitch..y * pitch + row_bytes];
        for x in 0..PANEL_WIDTH {
            // Qt's positive 90 degree transform: output(x,y) = input(y, H-1-x).
            let sx = y;
            let sy = IMAGE_HEIGHT - 1 - x;
            let si = (sy * IMAGE_WIDTH + sx) * 4;
            let di = x * 4;
            row[di..di + 4].copy_from_slice(&[source[si + 2], source[si + 1], source[si], 0]);
        }
    }
    Ok(())
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LeaseState {
    ready: bool,
    revoked: bool,
}
impl LeaseState {
    pub fn ready(&self) -> bool {
        self.ready && !self.revoked
    }
    pub fn mark_ready(&mut self) {
        if !self.revoked {
            self.ready = true;
        }
    }
    pub fn revoke(&mut self) {
        self.ready = false;
        self.revoked = true;
    }
}
