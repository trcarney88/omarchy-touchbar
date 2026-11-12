//! Dedicated owner thread for all blocking session-adapter work.
use super::quiescence::QuiescenceToken;
use super::session::SessionSnapshot;
use std::{
    sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const REFRESH_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Clone, Debug)]
pub enum SessionEvent {
    RefreshStarted {
        generation: u64,
    },
    Snapshot {
        generation: u64,
        snapshot: SessionSnapshot,
    },
    Failed {
        generation: u64,
    },
    QuiescenceReady {
        transaction: u64,
    },
    QuiescenceStopped {
        transaction: u64,
    },
}
#[derive(Debug)]
enum Command {
    Ack(u64),
    Park(u64),
    Stop(u64),
    CompleteSafe(u64),
    ForceStop,
}
#[derive(Default)]
struct Protocol {
    parked_txn: Option<u64>,
    complete_txn: Option<u64>,
    forced: bool,
}
impl Protocol {
    fn apply(&mut self, command: Command, generation: u64) -> Option<u64> {
        match command {
            Command::Park(txn) => {
                if self.parked_txn.is_none() {
                    self.parked_txn = Some(txn);
                }
            }
            Command::CompleteSafe(txn) if self.parked_txn == Some(txn) => {
                self.complete_txn = Some(txn)
            }
            Command::ForceStop => self.forced = true,
            Command::Stop(g) if g == generation => self.forced = true,
            _ => {}
        }
        if self.forced {
            return Some(0);
        }
        self.parked_txn
            .filter(|txn| self.complete_txn == Some(*txn))
    }
}

/// Source contract allows deterministic protocol tests without touching D-Bus or processes.
pub trait SessionSource: Send + 'static {
    fn begin_refresh(&mut self) -> Result<(), ()>;
    fn poll(&mut self, now: Instant) -> SessionSnapshot;
    fn acquire_inhibitor(&mut self) -> Result<(), ()>;
    fn acknowledge_panel_off(&mut self) -> bool;
}

pub struct SessionWorker {
    events: Receiver<SessionEvent>,
    commands: Sender<Command>,
    thread: Option<JoinHandle<()>>,
}
impl SessionWorker {
    #[cfg(all(target_os = "linux", feature = "linux-hardware"))]
    pub fn start() -> Result<Self, String> {
        Self::start_with(|| super::session::SessionAdapter::start().map_err(|e| e.to_string()))
    }
    pub fn start_with<F, S>(factory: F) -> Result<Self, String>
    where
        F: FnOnce() -> Result<S, String> + Send + 'static,
        S: SessionSource,
    {
        let (events_tx, events) = mpsc::channel();
        let (commands_tx, commands) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("touchmeter-session".into())
            .spawn(move || {
                let mut source = match factory() {
                    Ok(s) => s,
                    Err(_) => {
                        let _ = events_tx.send(SessionEvent::Failed { generation: 0 });
                        return;
                    }
                };
                let mut generation = 0u64;
                let mut stopped = false;
                let mut sleeping = false;
                let mut protocol = Protocol::default();
                let mut parked = false;
                let mut ready_sent = false;
                if source.acquire_inhibitor().is_err() {
                    let _ = events_tx.send(SessionEvent::Failed { generation: 0 });
                    return;
                }
                let mut next = Instant::now();
                loop {
                    loop {
                        match commands.try_recv() {
                            Ok(Command::Ack(g)) if g == generation => {
                                source.acknowledge_panel_off();
                            }
                            Ok(command) => {
                                if let Some(txn) = protocol.apply(command, generation) {
                                    stopped = true;
                                    if txn != 0 {
                                        let _ = events_tx.send(SessionEvent::QuiescenceStopped {
                                            transaction: txn,
                                        });
                                    }
                                }
                            }
                            Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
                        }
                    }
                    if stopped {
                        break;
                    }
                    if protocol.parked_txn.is_some() {
                        parked = true;
                    }
                    if parked {
                        if !ready_sent {
                            if let Some(transaction) = protocol.parked_txn {
                                let _ =
                                    events_tx.send(SessionEvent::QuiescenceReady { transaction });
                                ready_sent = true;
                            }
                        }
                        match commands.recv_timeout(Duration::from_secs(1)) {
                            Ok(command) => {
                                if let Some(txn) = protocol.apply(command, generation) {
                                    stopped = true;
                                    if txn != 0 {
                                        let _ = events_tx.send(SessionEvent::QuiescenceStopped {
                                            transaction: txn,
                                        });
                                    }
                                }
                            }
                            Err(RecvTimeoutError::Disconnected | RecvTimeoutError::Timeout) => {}
                        }
                        continue;
                    }
                    let now = Instant::now();
                    if now >= next {
                        generation = generation.wrapping_add(1);
                        let _ = events_tx.send(SessionEvent::RefreshStarted { generation });
                        let was_sleeping = sleeping;
                        let mut refresh_started = Instant::now();
                        if source.begin_refresh().is_err() {
                            let _ = events_tx.send(SessionEvent::Failed { generation });
                            next = Instant::now() + Duration::from_secs(1);
                            continue;
                        }
                        let mut snap = source.poll(Instant::now());
                        while snap.valid_until > Instant::now()
                            && !snap.allowed
                            && !snap.requires_panel_off
                        {
                            match commands.recv_timeout(Duration::from_millis(10)) {
                                Ok(Command::Ack(g)) if g == generation => {
                                    source.acknowledge_panel_off();
                                }
                                Ok(command) => {
                                    if protocol.apply(command, generation).is_some() {
                                        stopped = true;
                                        break;
                                    }
                                }
                                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {
                                }
                            }
                            if stopped {
                                break;
                            }
                            snap = source.poll(Instant::now());
                        }
                        if stopped {
                            break;
                        }
                        if let Some(txn) = protocol.parked_txn {
                            parked = true;
                            let _ =
                                events_tx.send(SessionEvent::QuiescenceReady { transaction: txn });
                            if protocol.complete_txn == Some(txn) {
                                stopped = true;
                                let _ = events_tx
                                    .send(SessionEvent::QuiescenceStopped { transaction: txn });
                                break;
                            }
                        }
                        if parked {
                            continue;
                        }
                        if snap.preparing_sleep || snap.preparing_shutdown {
                            snap.allowed = false;
                            snap.requires_panel_off = true;
                        }
                        sleeping = snap.preparing_sleep || snap.preparing_shutdown;
                        if was_sleeping && !sleeping {
                            if source.acquire_inhibitor().is_err() {
                                let _ = events_tx.send(SessionEvent::Failed { generation });
                                next = Instant::now() + Duration::from_secs(1);
                                continue;
                            }
                            generation = generation.wrapping_add(1);
                            let _ = events_tx.send(SessionEvent::RefreshStarted { generation });
                            refresh_started = Instant::now();
                            if source.begin_refresh().is_err() {
                                let _ = events_tx.send(SessionEvent::Failed { generation });
                                next = Instant::now() + Duration::from_secs(1);
                                continue;
                            }
                            snap = source.poll(Instant::now());
                            while snap.valid_until > Instant::now()
                                && !snap.allowed
                                && !snap.requires_panel_off
                            {
                                match commands.recv_timeout(Duration::from_millis(10)) {
                                    Ok(Command::Ack(g)) if g == generation => {
                                        source.acknowledge_panel_off();
                                    }
                                    Ok(command) => {
                                        if protocol.apply(command, generation).is_some() {
                                            stopped = true;
                                            break;
                                        }
                                    }
                                    Err(
                                        RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected,
                                    ) => {}
                                }
                                if stopped {
                                    break;
                                }
                                snap = source.poll(Instant::now());
                            }
                            if stopped {
                                break;
                            }
                            if snap.preparing_sleep || snap.preparing_shutdown {
                                snap.allowed = false;
                                snap.requires_panel_off = true;
                            }
                            sleeping = snap.preparing_sleep || snap.preparing_shutdown;
                        }
                        let _ = events_tx.send(SessionEvent::Snapshot {
                            generation,
                            snapshot: snap,
                        });
                        next = refresh_started + REFRESH_INTERVAL;
                    }
                    thread::sleep(Duration::from_millis(10));
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            events,
            commands: commands_tx,
            thread: Some(thread),
        })
    }
    pub fn poll(&self) -> Option<SessionEvent> {
        self.events.try_recv().ok()
    }
    pub fn recv_timeout(&self, timeout: Duration) -> Result<SessionEvent, mpsc::RecvTimeoutError> {
        self.events.recv_timeout(timeout)
    }
    pub fn drain(&self) -> Vec<SessionEvent> {
        let mut v = Vec::new();
        while let Some(e) = self.poll() {
            v.push(e);
        }
        v
    }
    pub fn acknowledge_panel_off(&self, generation: u64) {
        let _ = self.commands.send(Command::Ack(generation));
    }
    pub fn stop_after_panel_off(&self, generation: u64) {
        let _ = self.commands.send(Command::Stop(generation));
    }
    pub fn begin_quiescence(&self, transaction: u64) {
        let _ = self.commands.send(Command::Park(transaction));
    }
    pub fn complete_quiescence(&self, token: &QuiescenceToken) {
        let _ = self
            .commands
            .send(Command::CompleteSafe(token.transaction()));
    }
    pub fn force_stop(&self) {
        let _ = self.commands.send(Command::ForceStop);
    }
    /// Returns false if worker is still blocked past the caller's bound. Panel should already be off.
    pub fn stop_and_join(&mut self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        if let Some(handle) = self.thread.take() {
            while !handle.is_finished() {
                if Instant::now() >= deadline {
                    self.thread = Some(handle);
                    return false;
                }
                thread::sleep(Duration::from_millis(5));
            }
            let _ = handle.join();
        }
        true
    }
}

#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
impl SessionSource for super::session::SessionAdapter {
    fn begin_refresh(&mut self) -> Result<(), ()> {
        if super::session::SessionAdapter::begin_refresh(self) {
            Ok(())
        } else {
            Err(())
        }
    }
    fn poll(&mut self, now: Instant) -> SessionSnapshot {
        super::session::SessionAdapter::poll(self, now)
    }
    fn acquire_inhibitor(&mut self) -> Result<(), ()> {
        super::session::SessionAdapter::acquire_inhibitor(self).map_err(|_| ())
    }
    fn acknowledge_panel_off(&mut self) -> bool {
        super::session::SessionAdapter::acknowledge_panel_off(self)
    }
}
impl Drop for SessionWorker {
    fn drop(&mut self) {
        if let Some(h) = self.thread.take() {
            if h.is_finished() {
                let _ = h.join();
            }
        }
    }
}
