use crate::{jobs::Job, live::LiveBackend};
use chrono::{DateTime, Utc};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

const REFRESH: Duration = Duration::from_secs(300);
const RELOAD: Duration = Duration::from_secs(15);
const TIMEOUT: Duration = Duration::from_secs(60);
const OUTPUT_CAP: usize = 1024 * 1024;

/// Explicitly-started coordinator for collector jobs and record reloads.
pub struct LiveWorker {
    program: PathBuf,
    job: Option<Job>,
    generation: u64,
    next_refresh: Option<Instant>,
    next_reload: Option<Instant>,
    last_snapshot: Option<crate::live::LiveSnapshot>,
    next_credential_request: Option<Instant>,
    shutdown: bool,
    paused: bool,
}
impl LiveWorker {
    pub fn new(backend: &LiveBackend, program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            job: None,
            generation: backend.generation(),
            next_refresh: None,
            next_reload: None,
            last_snapshot: None,
            next_credential_request: None,
            shutdown: false,
            paused: true,
        }
    }
    /// Call only from the application coordinator; construction starts no subprocess.
    pub fn tick(
        &mut self,
        backend: &mut LiveBackend,
        instant: Instant,
        now: DateTime<Utc>,
    ) -> Option<&crate::live::LiveSnapshot> {
        if self.shutdown || self.paused || backend.test_mode {
            self.cancel();
            self.next_refresh = None;
            self.next_reload = None;
            self.next_credential_request = None;
            return None;
        }
        if self.generation != backend.generation() {
            self.cancel();
            self.generation = backend.generation();
            self.next_refresh = None;
        }
        if self.next_reload.is_none_or(|t| instant >= t) {
            self.last_snapshot = Some(backend.poll(now));
            self.next_reload = Some(instant + RELOAD);
        }
        if let Some(job) = self.job.as_mut() {
            if let Some(result) = job.poll() {
                if result.generation == self.generation
                    && result.error != Some(crate::jobs::JobError::Cancelled)
                {
                    backend.complete(
                        self.generation,
                        result.error.is_some() || result.status != Some(0),
                    );
                }
                self.job = None;
            }
        }
        if self.job.is_none() && self.next_refresh.is_none_or(|t| instant >= t) {
            let Some(request) = backend.refresh_request() else {
                return self.last_snapshot.as_ref();
            };
            let args = request.argv.into_iter().skip(1).collect::<Vec<_>>();
            self.job = Job::start(
                self.program.to_string_lossy().as_ref(),
                args,
                std::iter::empty::<(String, String)>(),
                TIMEOUT,
                OUTPUT_CAP,
                request.generation,
            )
            .ok();
            self.next_refresh = Some(instant + REFRESH);
        }
        self.last_snapshot.as_ref()
    }
    pub fn pause(&mut self) {
        self.paused = true;
        if let Some(job) = self.job.as_mut() {
            job.cancel();
        }
    }
    pub fn resume(&mut self) {
        if !self.shutdown {
            self.paused = false;
            self.next_refresh = None;
            self.next_reload = None;
        }
    }
    pub fn select(&mut self, backend: &mut LiveBackend, id: &str) -> bool {
        let ok = backend.select(id);
        if ok {
            self.cancel();
            self.generation = backend.generation();
            self.next_refresh = None;
            self.next_reload = None;
        }
        ok
    }
    pub fn set_test_mode(&mut self, backend: &mut LiveBackend, on: bool) {
        if self.shutdown {
            return;
        }
        backend.set_test_mode(on);
        if on {
            self.cancel();
            self.next_refresh = None;
            self.next_reload = None;
        } else {
            self.generation = backend.generation();
            self.next_refresh = None;
            self.next_reload = None;
        }
    }
    /// Permanently stop worker activity; unlike test mode this cannot be resumed.
    pub fn shutdown(&mut self) {
        if self.shutdown {
            return;
        }
        self.shutdown = true;
        if let Some(job) = self.job.as_mut() {
            if job.cancel_and_wait(Duration::from_millis(250)).is_some() {
                self.job = None;
            }
        }
        self.next_refresh = None;
        self.next_reload = None;
        self.next_credential_request = None;
    }
    pub fn update_availability(
        &mut self,
        backend: &mut LiveBackend,
        map: std::collections::HashMap<String, bool>,
    ) {
        backend.update_availability(map);
        self.next_reload = None;
    }
    /// Credential discovery is delegated to the separate read-only worker.
    pub fn credential_request_due(&mut self, instant: Instant, testing: bool) -> bool {
        if testing {
            self.next_credential_request = None;
            return false;
        }
        if self.next_credential_request.is_none_or(|t| instant >= t) {
            self.next_credential_request = Some(instant + RELOAD);
            true
        } else {
            false
        }
    }
    fn cancel(&mut self) {
        if let Some(mut job) = self.job.take() {
            job.cancel();
        }
    }
    pub fn last_snapshot(&self) -> Option<&crate::live::LiveSnapshot> {
        self.last_snapshot.as_ref()
    }
    pub fn intervals() -> (Duration, Duration) {
        (REFRESH, RELOAD)
    }
}
impl Drop for LiveWorker {
    fn drop(&mut self) {
        self.cancel()
    }
}
