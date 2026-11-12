//! Bounded shutdown transaction driver. This module owns neither worker nor actuator.
use super::quiescence::{self, Actuator, QuiescenceToken};
use std::time::Duration;

pub const PARK_TIMEOUT: Duration = Duration::from_millis(250);
pub const JOIN_TIMEOUT: Duration = Duration::from_millis(500);
pub const FORCE_TIMEOUT: Duration = Duration::from_millis(500);

pub trait Worker {
    fn request_park(&mut self, transaction: u64) -> Result<(), String>;
    fn wait_ready(&mut self, transaction: u64, timeout: Duration) -> Result<(), String>;
    fn complete_and_join(
        &mut self,
        token: &QuiescenceToken,
        timeout: Duration,
    ) -> Result<(), String>;
    fn force_stop_and_join(&mut self, timeout: Duration) -> Result<(), String>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Running,
    Quiescing,
    FaultHeld,
    SafeStopped,
    ForcedReleased,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    OrdinaryStop,
    Retry,
    Force,
    Allow,
    TimeAdvanced,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Noop(State),
    Held,
    SafeStopped,
    ForcedReleased,
}

pub struct ShutdownTransaction {
    state: State,
    transaction: u64,
    park_ready: bool,
    pub last_failure: Option<String>,
    pub attempt_count: u32,
}

impl ShutdownTransaction {
    pub fn fault_held(transaction: u64, report: String) -> Self {
        Self {
            state: State::FaultHeld,
            transaction,
            park_ready: false,
            last_failure: Some(report),
            attempt_count: 1,
        }
    }
    pub fn new(transaction: u64) -> Self {
        Self {
            state: State::Running,
            transaction,
            park_ready: false,
            last_failure: None,
            attempt_count: 0,
        }
    }
    pub fn state(&self) -> State {
        self.state
    }
    pub fn transaction(&self) -> u64 {
        self.transaction
    }
    pub fn park_ready(&self) -> bool {
        self.park_ready
    }

    pub fn handle<A: Actuator, W: Worker>(
        &mut self,
        event: Event,
        actuator: &mut A,
        worker: &mut W,
    ) -> Outcome {
        match event {
            Event::OrdinaryStop if self.state == State::Running => {
                self.attempt_once(actuator, worker)
            }
            Event::Retry if self.state == State::FaultHeld => self.attempt_once(actuator, worker),
            Event::Force if self.state == State::FaultHeld => {
                match worker.force_stop_and_join(FORCE_TIMEOUT) {
                    Ok(()) => {
                        self.state = State::ForcedReleased;
                        Outcome::ForcedReleased
                    }
                    Err(e) => {
                        self.last_failure = Some(e);
                        Outcome::Held
                    }
                }
            }
            _ => Outcome::Noop(self.state),
        }
    }

    fn attempt_once<A: Actuator, W: Worker>(
        &mut self,
        actuator: &mut A,
        worker: &mut W,
    ) -> Outcome {
        self.state = State::Quiescing;
        self.attempt_count += 1;
        let park_error = worker.request_park(self.transaction).err();
        // Always execute the complete actuator barrier, even if park request failed.
        let token = match quiescence::attempt(actuator, self.transaction) {
            Ok(token) => token,
            Err(e) => {
                self.last_failure = Some(format!(
                    "quiescence failed: {e:?}; park request: {park_error:?}"
                ));
                self.state = State::FaultHeld;
                return Outcome::Held;
            }
        };
        if !self.park_ready {
            if let Err(e) = worker.wait_ready(self.transaction, PARK_TIMEOUT) {
                self.last_failure = Some(format!("worker park not ready: {e}"));
                self.state = State::FaultHeld;
                return Outcome::Held;
            }
            self.park_ready = true;
        }
        if let Err(e) = worker.complete_and_join(&token, JOIN_TIMEOUT) {
            self.last_failure = Some(format!("worker completion/join failed: {e}"));
            self.state = State::FaultHeld;
            return Outcome::Held;
        }
        self.last_failure = None;
        self.state = State::SafeStopped;
        Outcome::SafeStopped
    }
}
