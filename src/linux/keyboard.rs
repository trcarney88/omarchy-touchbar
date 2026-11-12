//! Virtual-keyboard protocol adapter and function-key helpers.
#![allow(dead_code)]
use std::collections::BTreeSet;

#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
mod adapter {
    use super::PressedKeys;
    use std::{
        fs::File,
        io::{Seek, SeekFrom, Write},
        os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd},
        time::Instant,
    };
    use xkbcommon::xkb::{Context, Keymap};

    mod virtual_keyboard {
        use wayland_client;
        use wayland_client::protocol::*;
        pub mod __interfaces {
            use wayland_client;
            use wayland_client::backend as wayland_backend;
            use wayland_client::protocol::__interfaces::*;
            wayland_scanner::generate_interfaces!("./protocols/virtual-keyboard-unstable-v1.xml");
        }
        use self::__interfaces::*;
        wayland_scanner::generate_client_code!("./protocols/virtual-keyboard-unstable-v1.xml");
    }
    use virtual_keyboard::{
        zwp_virtual_keyboard_manager_v1::{self, ZwpVirtualKeyboardManagerV1},
        zwp_virtual_keyboard_v1::{self, ZwpVirtualKeyboardV1},
    };
    use wayland_protocols::ext::idle_notify::v1::client::{
        ext_idle_notification_v1::{self, ExtIdleNotificationV1},
        ext_idle_notifier_v1::{self, ExtIdleNotifierV1},
    };

    use std::{os::fd::BorrowedFd, time::Duration};
    use wayland_client::{
        protocol::{wl_callback, wl_registry, wl_seat},
        Connection, Dispatch, EventQueue, Proxy, QueueHandle,
    };

    fn callback_matches(completed: Option<u32>, expected: u32) -> bool {
        completed == Some(expected)
    }
    fn barrier_expired(now: Instant, deadline: Instant) -> bool {
        now >= deadline
    }

    struct State {
        seat: Option<wl_seat::WlSeat>,
        manager: Option<ZwpVirtualKeyboardManagerV1>,
        keyboard: Option<ZwpVirtualKeyboardV1>,
        idle_notifier: Option<ExtIdleNotifierV1>,
        idle_notification: Option<ExtIdleNotificationV1>,
        idle: Option<bool>,
        idle_changed: Option<bool>,
        seat_name: Option<u32>,
        manager_name: Option<u32>,
        sync_done: bool,
        barrier_completed: Option<u32>,
        next_barrier: u32,
        terminal: Option<String>,
        barrier_timed_out: bool,
        keys: PressedKeys,
        started: Instant,
    }
    impl Default for State {
        fn default() -> Self {
            Self {
                seat: None,
                manager: None,
                keyboard: None,
                idle_notifier: None,
                idle_notification: None,
                idle: None,
                idle_changed: None,
                seat_name: None,
                manager_name: None,
                sync_done: false,
                barrier_completed: None,
                next_barrier: 1,
                terminal: None,
                barrier_timed_out: false,
                keys: PressedKeys::default(),
                started: Instant::now(),
            }
        }
    }
    macro_rules! noop_dispatch {
        ($ty:ty,$event:ty) => {
            impl Dispatch<$ty, ()> for State {
                fn event(
                    _: &mut Self,
                    _: &$ty,
                    _: $event,
                    _: &(),
                    _: &Connection,
                    _: &QueueHandle<Self>,
                ) {
                }
            }
        };
    }
    impl Dispatch<wl_registry::WlRegistry, ()> for State {
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
                } if interface == wl_seat::WlSeat::interface().name => {
                    if s.seat.is_some() || version < 7 {
                        s.terminal = Some("Missing or ambiguous suitable Wayland seat".into());
                        return;
                    }
                    s.seat_name = Some(name);
                    s.seat = Some(p.bind(name, 7.min(version), q, ()));
                }
                wl_registry::Event::Global {
                    name,
                    interface,
                    version,
                } if interface == ZwpVirtualKeyboardManagerV1::interface().name => {
                    if s.manager.is_some() || version < 1 {
                        s.terminal = Some("Missing or ambiguous virtual keyboard manager".into());
                        return;
                    }
                    s.manager_name = Some(name);
                    s.manager = Some(p.bind(name, 1, q, ()));
                }
                wl_registry::Event::Global {
                    name,
                    interface,
                    version,
                } if interface == ExtIdleNotifierV1::interface().name => {
                    if s.idle_notifier.is_some() {
                        return;
                    }
                    s.idle_notifier = Some(p.bind(name, version.min(2), q, ()))
                }
                wl_registry::Event::GlobalRemove { name }
                    if Some(name) == s.seat_name || Some(name) == s.manager_name =>
                {
                    s.terminal = Some("Virtual keyboard global removed".into())
                }
                _ => {}
            }
        }
    }
    noop_dispatch!(wl_seat::WlSeat, wl_seat::Event);
    noop_dispatch!(
        ZwpVirtualKeyboardManagerV1,
        zwp_virtual_keyboard_manager_v1::Event
    );
    noop_dispatch!(ZwpVirtualKeyboardV1, zwp_virtual_keyboard_v1::Event);
    noop_dispatch!(ExtIdleNotifierV1, ext_idle_notifier_v1::Event);
    impl Dispatch<ExtIdleNotificationV1, ()> for State {
        fn event(
            s: &mut Self,
            _: &ExtIdleNotificationV1,
            e: ext_idle_notification_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            let value = match e {
                ext_idle_notification_v1::Event::Idled => Some(true),
                ext_idle_notification_v1::Event::Resumed => Some(false),
                _ => None,
            };
            if let Some(value) = value {
                if s.idle != Some(value) {
                    s.idle = Some(value);
                    s.idle_changed = Some(value)
                }
            }
        }
    }
    impl Dispatch<wl_callback::WlCallback, u32> for State {
        fn event(
            s: &mut Self,
            _: &wl_callback::WlCallback,
            e: wl_callback::Event,
            data: &u32,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let wl_callback::Event::Done { .. } = e {
                if *data == 0 {
                    s.sync_done = true
                } else {
                    s.barrier_completed = Some(*data)
                }
            }
        }
    }

    pub struct Adapter {
        connection: Connection,
        queue: EventQueue<State>,
        state: State,
        deadline: Instant,
        keymap: Vec<u8>,
        keymap_fd: Option<OwnedFd>,
    }
    impl Adapter {
        pub fn start(timeout: Duration) -> Result<Self, String> {
            let keymap = us_keymap()?;
            let keymap_fd = Some(keymap_fd(&keymap)?);
            let connection = Connection::connect_to_env().map_err(|e| e.to_string())?;
            let queue = connection.new_event_queue();
            let qh = queue.handle();
            let state = State::default();
            connection.display().get_registry(&qh, ());
            connection.display().sync(&qh, 0);
            Ok(Self {
                connection,
                queue,
                state,
                deadline: Instant::now() + timeout,
                keymap,
                keymap_fd,
            })
        }
        pub fn pump(&mut self, timeout: Duration) -> Result<(), String> {
            let until = if self.ready() {
                Instant::now() + timeout
            } else {
                self.deadline.min(Instant::now() + timeout)
            };
            loop {
                self.queue
                    .dispatch_pending(&mut self.state)
                    .map_err(|e| e.to_string())?;
                if let Some(e) = &self.state.terminal {
                    return Err(e.clone());
                }
                if self.state.sync_done && !self.state.keyboard.is_some() {
                    let seat = self.state.seat.as_ref().ok_or("No suitable Wayland seat")?;
                    let manager = self
                        .state
                        .manager
                        .as_ref()
                        .ok_or("No virtual keyboard manager")?;
                    let qh = self.queue.handle();
                    let kb = manager.create_virtual_keyboard(seat, &qh, ());
                    let fd = self.keymap_fd.take().ok_or("Keymap FD already sent")?;
                    kb.keymap(1, fd.as_fd(), self.keymap.len() as u32);
                    kb.modifiers(0, 0, 0, 0);
                    self.state.keyboard = Some(kb);
                    if let Some(notifier) = self.state.idle_notifier.as_ref() {
                        let notification = if notifier.version() >= 2 {
                            notifier.get_input_idle_notification(120_000, seat, &qh, ())
                        } else {
                            notifier.get_idle_notification(120_000, seat, &qh, ())
                        };
                        self.state.idle_notification = Some(notification);
                    }
                }
                if !self.ready() && Instant::now() >= self.deadline {
                    return Err("Timed out waiting for virtual keyboard globals".into());
                }
                if let Err(e) = self.connection.flush() {
                    if !matches!(e,wayland_client::backend::WaylandError::Io(ref io) if io.kind()==std::io::ErrorKind::WouldBlock)
                    {
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
                let ms = if self.ready() {
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
                    return Err(e.to_string());
                }
                if p.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
                    drop(guard);
                    return Err("Wayland connection failed".into());
                }
                if n > 0 {
                    if let Err(e) = guard.read() {
                        if !matches!(e,wayland_client::backend::WaylandError::Io(ref io) if io.kind()==std::io::ErrorKind::WouldBlock)
                        {
                            return Err(e.to_string());
                        }
                    }
                } else {
                    drop(guard)
                }
                if Instant::now() >= until {
                    return Ok(());
                }
            }
        }
        pub fn keymap_fd(&self) -> Option<BorrowedFd<'_>> {
            self.keymap_fd.as_ref().map(AsFd::as_fd)
        }
        pub fn socket_fd(&self) -> BorrowedFd<'_> {
            self.connection.as_fd()
        }
        pub fn ready(&self) -> bool {
            self.state.keyboard.is_some()
        }
        /// None means the compositor did not advertise ext-idle-notify; callers should use a conservative local timer.
        pub fn idle_available(&self) -> bool {
            self.state.idle_notification.is_some()
        }
        pub fn take_idle_changed(&mut self) -> Option<bool> {
            self.state.idle_changed.take()
        }
        pub fn function_key(&mut self, n: u8, pressed: bool) -> Result<bool, String> {
            if pressed && self.state.barrier_timed_out {
                return Err("Virtual-keyboard release barrier timed out; input disabled".into());
            }
            let Some((code, down)) = self.state.keys.transition(n, pressed) else {
                return Ok(false);
            };
            let kb = self
                .state
                .keyboard
                .as_ref()
                .ok_or("Virtual keyboard is not ready")?;
            kb.key(self.state.keys_time(), code, u32::from(down));
            Ok(true)
        }
        pub fn release_all(&mut self) -> Result<(), String> {
            let releases = self.state.keys.release_all();
            let kb = self
                .state
                .keyboard
                .as_ref()
                .ok_or("Virtual keyboard is not ready")?;
            let time = self.state.keys_time();
            for code in releases {
                kb.key(time, code, 0)
            }
            Ok(())
        }
        /// Releases all virtual keys and waits until the compositor processes the requests.
        /// Any timeout or connection error is terminal; callers must not acknowledge sleep.
        pub fn release_all_and_sync(&mut self, timeout: Duration) -> Result<(), String> {
            self.release_and_sync(timeout, false)
        }
        /// Retry the release barrier on the existing connection after a timeout.
        pub fn recover_release_and_sync(&mut self, timeout: Duration) -> Result<(), String> {
            self.release_and_sync(timeout, true)
        }
        fn release_and_sync(&mut self, timeout: Duration, recovery: bool) -> Result<(), String> {
            if self.state.terminal.is_some() {
                return Err(self.state.terminal.clone().unwrap());
            }
            if self.state.barrier_timed_out && !recovery {
                return Err("Virtual-keyboard release barrier timed out; recovery required".into());
            }
            let deadline = Instant::now() + timeout;
            let kb = self
                .state
                .keyboard
                .as_ref()
                .ok_or("Virtual keyboard is not ready")?;
            let timestamp = self.state.keys_time();
            for code in self.state.keys.release_all() {
                kb.key(timestamp, code, 0);
            }
            kb.modifiers(0, 0, 0, 0);
            let sequence = self.state.next_barrier.max(1);
            self.state.next_barrier = sequence.wrapping_add(1).max(1);
            self.state.barrier_completed = None;
            let qh = self.queue.handle();
            self.connection.display().sync(&qh, sequence);
            loop {
                self.queue.dispatch_pending(&mut self.state).map_err(|e| {
                    self.state.terminal = Some(e.to_string());
                    e.to_string()
                })?;
                if let Some(error) = &self.state.terminal {
                    return Err(error.clone());
                }
                if callback_matches(self.state.barrier_completed, sequence) {
                    return Ok(());
                }
                if barrier_expired(Instant::now(), deadline) {
                    let error = "Timed out waiting for virtual-keyboard release barrier".to_owned();
                    self.state.barrier_timed_out = true;
                    return Err(error);
                }
                if let Err(e) = self.connection.flush() {
                    if !matches!(e, wayland_client::backend::WaylandError::Io(ref io) if io.kind() == std::io::ErrorKind::WouldBlock)
                    {
                        self.state.terminal = Some(e.to_string());
                        return Err(e.to_string());
                    }
                }
                let guard = match self.connection.prepare_read() {
                    Some(g) => g,
                    None => continue,
                };
                let mut p = libc::pollfd {
                    fd: self.connection.as_fd().as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                };
                let ms = deadline
                    .saturating_duration_since(Instant::now())
                    .as_millis()
                    .max(1)
                    .min(i32::MAX as u128) as i32;
                let result = unsafe { libc::poll(&mut p, 1, ms) };
                if result < 0 {
                    let e = std::io::Error::last_os_error();
                    drop(guard);
                    if e.kind() == std::io::ErrorKind::Interrupted {
                        continue;
                    }
                    self.state.terminal = Some(e.to_string());
                    return Err(e.to_string());
                }
                if p.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
                    drop(guard);
                    self.state.terminal = Some("Wayland connection failed".into());
                    return Err("Wayland connection failed".into());
                }
                if result > 0 {
                    if let Err(e) = guard.read() {
                        if !matches!(e,wayland_client::backend::WaylandError::Io(ref io) if io.kind()==std::io::ErrorKind::WouldBlock)
                        {
                            self.state.terminal = Some(e.to_string());
                            return Err(e.to_string());
                        }
                    }
                } else {
                    drop(guard)
                }
            }
        }
        pub fn disconnect(&mut self) {
            self.state.terminal = Some("Virtual keyboard disconnected".into());
        }
    }

    #[cfg(test)]
    mod barrier_tests {
        use super::*;
        #[test]
        fn barriers_require_the_matching_callback_and_respect_deadline() {
            assert!(!callback_matches(Some(1), 2));
            assert!(callback_matches(Some(2), 2));
            let now = Instant::now();
            assert!(!barrier_expired(now, now + Duration::from_millis(1)));
            assert!(barrier_expired(now, now));
        }
    }
    impl State {
        fn keys_time(&self) -> u32 {
            self.started.elapsed().as_millis() as u32
        }
    }
    impl Drop for Adapter {
        fn drop(&mut self) {
            let _ = self.release_all();
            if let Some(k) = self.state.keyboard.take() {
                k.destroy()
            }
            if let Some(notification) = self.state.idle_notification.take() {
                notification.destroy()
            }
            if let Some(notifier) = self.state.idle_notifier.take() {
                notifier.destroy()
            }
            let _ = self.connection.flush();
        }
    }

    pub fn us_keymap() -> Result<Vec<u8>, String> {
        let context = Context::new(xkbcommon::xkb::CONTEXT_NO_FLAGS);
        let keymap = Keymap::new_from_names(
            &context,
            "",
            "",
            "us",
            "",
            None,
            xkbcommon::xkb::KEYMAP_COMPILE_NO_FLAGS,
        )
        .ok_or_else(|| "could not compile US XKB keymap".to_owned())?;
        let mut bytes = keymap
            .get_as_string(xkbcommon::xkb::KEYMAP_FORMAT_TEXT_V1)
            .into_bytes();
        if bytes.last() != Some(&0) {
            bytes.push(0);
        }
        Ok(bytes)
    }

    pub fn keymap_fd(bytes: &[u8]) -> Result<OwnedFd, String> {
        let name = std::ffi::CString::new("touchmeter-keymap").unwrap();
        let raw = unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC) };
        if raw < 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let owned = unsafe { OwnedFd::from_raw_fd(raw) };
        let mut file = File::from(owned);
        file.write_all(bytes).map_err(|e| e.to_string())?;
        file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        use std::os::fd::IntoRawFd;
        Ok(unsafe { OwnedFd::from_raw_fd(file.into_raw_fd()) })
    }

    #[allow(dead_code)]
    pub struct KeyboardState {
        keys: PressedKeys,
        started: Instant,
    }
    impl KeyboardState {
        pub fn new() -> Self {
            Self {
                keys: PressedKeys::default(),
                started: Instant::now(),
            }
        }
        pub fn event(&mut self, number: u8, pressed: bool) -> Option<(u32, u32)> {
            self.keys.transition(number, pressed).map(|(code, down)| {
                (
                    self.started.elapsed().as_millis() as u32,
                    code | (u32::from(!down) << 31),
                )
            })
        }
        pub fn releases(&mut self) -> Vec<(u32, u32)> {
            let time = self.started.elapsed().as_millis() as u32;
            self.keys
                .release_all()
                .into_iter()
                .map(|code| (time, code | (1 << 31)))
                .collect()
        }
    }
}

#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
pub use adapter::Adapter;

/// Linux evdev keycode for a function key, restricted to the F1--F12 range.
pub fn function_key_code(number: u8) -> Option<u32> {
    match number {
        1..=10 => Some(58 + u32::from(number)),
        11..=12 => Some(76 + u32::from(number)),
        _ => None,
    }
}

/// Tracks only F keys and makes duplicate press/release requests idempotent.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PressedKeys(BTreeSet<u8>);

impl PressedKeys {
    pub fn transition(&mut self, number: u8, pressed: bool) -> Option<(u32, bool)> {
        let code = function_key_code(number)?;
        let changed = if pressed {
            self.0.insert(number)
        } else {
            self.0.remove(&number)
        };
        changed.then_some((code, pressed))
    }
    pub fn release_all(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.0)
            .into_iter()
            .filter_map(function_key_code)
            .collect()
    }
    pub fn is_pressed(&self, number: u8) -> bool {
        self.0.contains(&number)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_function_keys_and_balanced_transitions() {
        assert_eq!(function_key_code(1), Some(59));
        assert_eq!(function_key_code(12), Some(88));
        assert_eq!(function_key_code(0), None);
        assert_eq!(function_key_code(13), None);
        let mut keys = PressedKeys::default();
        assert_eq!(keys.transition(2, true), Some((60, true)));
        assert_eq!(keys.transition(2, true), None);
        assert_eq!(keys.release_all(), vec![60]);
        assert_eq!(keys.release_all(), Vec::<u32>::new());
    }
}
