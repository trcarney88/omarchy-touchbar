//! Explicit, bounded, shell-free Unix subprocess jobs.
use std::{
    io::{self, Read},
    os::{fd::AsRawFd, unix::process::CommandExt},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU8, Ordering},
        mpsc::{self, Receiver, TryRecvError},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Debug)]
pub struct JobResult {
    pub generation: u64,
    pub status: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub error: Option<JobError>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobError {
    Spawn,
    Timeout,
    OutputLimit,
    Cancelled,
    Io,
}
pub struct Job {
    generation: u64,
    rx: Receiver<JobResult>,
    cancel: Arc<AtomicBool>,
    dispatch: Arc<AtomicU8>,
    complete: bool,
}
impl Job {
    pub fn start<I, S, E, K, V>(
        program: &str,
        args: I,
        env: E,
        timeout: Duration,
        max_output: usize,
        generation: u64,
    ) -> io::Result<Self>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
        E: IntoIterator<Item = (K, V)>,
        K: AsRef<std::ffi::OsStr>,
        V: AsRef<std::ffi::OsStr>,
    {
        Self::start_with_hooks(
            program,
            args,
            env,
            timeout,
            max_output,
            generation,
            || {},
            || {},
        )
    }
    /// Hook runs before the dispatch authorization lock; useful for deterministic callers/tests.
    pub fn start_with_pre_spawn_hook<I, S, E, K, V, F>(
        program: &str,
        args: I,
        env: E,
        timeout: Duration,
        max_output: usize,
        generation: u64,
        hook: F,
    ) -> io::Result<Self>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
        E: IntoIterator<Item = (K, V)>,
        K: AsRef<std::ffi::OsStr>,
        V: AsRef<std::ffi::OsStr>,
        F: FnOnce() + Send + 'static,
    {
        Self::start_with_hooks(
            program,
            args,
            env,
            timeout,
            max_output,
            generation,
            hook,
            || {},
        )
    }
    /// Test seam to pause immediately after dispatch commitment but before OS spawn.
    pub fn start_with_hooks<I, S, E, K, V, F, G>(
        program: &str,
        args: I,
        env: E,
        timeout: Duration,
        max_output: usize,
        generation: u64,
        precommit: F,
        committed: G,
    ) -> io::Result<Self>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
        E: IntoIterator<Item = (K, V)>,
        K: AsRef<std::ffi::OsStr>,
        V: AsRef<std::ffi::OsStr>,
        F: FnOnce() + Send + 'static,
        G: FnOnce() + Send + 'static,
    {
        let args: Vec<_> = args.into_iter().map(|x| x.as_ref().to_owned()).collect();
        let env: Vec<_> = env
            .into_iter()
            .map(|(k, v)| (k.as_ref().to_owned(), v.as_ref().to_owned()))
            .collect();
        let program = std::ffi::OsString::from(program);
        let (tx, rx) = mpsc::channel();
        let state = Arc::new(AtomicBool::new(false));
        let worker_state = state.clone();
        let dispatch = Arc::new(AtomicU8::new(0));
        let worker_dispatch = dispatch.clone();
        thread::Builder::new()
            .name("touchmeter-job".into())
            .spawn(move || {
                precommit();
                if worker_dispatch
                    .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
                    .is_err()
                {
                    let _ = tx.send(result(
                        generation,
                        None,
                        vec![],
                        vec![],
                        Some(JobError::Cancelled),
                    ));
                    return;
                }
                committed();
                if worker_state.load(Ordering::Acquire) {
                    let _ = tx.send(result(
                        generation,
                        None,
                        vec![],
                        vec![],
                        Some(JobError::Cancelled),
                    ));
                    return;
                }
                let mut cmd = Command::new(program);
                cmd.args(args)
                    .envs(env)
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped());
                unsafe {
                    cmd.pre_exec(|| {
                        if libc::setpgid(0, 0) == 0 {
                            Ok(())
                        } else {
                            Err(io::Error::last_os_error())
                        }
                    });
                }
                let mut child = match cmd.spawn() {
                    Ok(c) => c,
                    Err(_) => {
                        let _ = tx.send(result(
                            generation,
                            None,
                            vec![],
                            vec![],
                            Some(JobError::Spawn),
                        ));
                        return;
                    }
                };
                let pid = child.id() as i32;
                if worker_state.load(Ordering::Acquire) {
                    unsafe {
                        libc::kill(-(pid as i32), libc::SIGKILL);
                    }
                }
                let mut out = child.stdout.take().unwrap();
                let mut err = child.stderr.take().unwrap();
                let mut fault = false;
                for fd in [out.as_raw_fd(), err.as_raw_fd()] {
                    unsafe {
                        let f = libc::fcntl(fd, libc::F_GETFL);
                        if f < 0 || libc::fcntl(fd, libc::F_SETFL, f | libc::O_NONBLOCK) < 0 {
                            fault = true;
                        }
                    }
                }
                let deadline = Instant::now() + timeout;
                let mut stdout = Vec::new();
                let mut stderr = Vec::new();
                let mut out_eof = false;
                let mut err_eof = false;
                let mut error = if fault { Some(JobError::Io) } else { None };
                let mut exit_at = None;
                loop {
                    if worker_state.load(Ordering::Acquire) {
                        error = Some(JobError::Cancelled);
                        unsafe {
                            libc::kill(-pid, libc::SIGKILL);
                        }
                    }
                    if Instant::now() >= deadline {
                        error.get_or_insert(JobError::Timeout);
                        unsafe {
                            libc::kill(-pid, libc::SIGKILL);
                        }
                    }
                    if fault {
                        let _ = child.kill();
                        unsafe {
                            libc::kill(-pid, libc::SIGKILL);
                        }
                    }
                    let mut buf = [0u8; 4096];
                    if !out_eof {
                        match read_pipe(&mut out, &mut buf) {
                            Ok(None) => out_eof = true,
                            Ok(Some(n)) => {
                                if stdout.len().saturating_add(stderr.len()).saturating_add(n)
                                    > max_output
                                {
                                    error = Some(JobError::OutputLimit);
                                    unsafe {
                                        libc::kill(-pid, libc::SIGKILL);
                                    }
                                } else {
                                    stdout.extend_from_slice(&buf[..n]);
                                }
                            }
                            Err(e)
                                if e.kind() == io::ErrorKind::WouldBlock
                                    || e.kind() == io::ErrorKind::Interrupted => {}
                            Err(_) => {
                                error.get_or_insert(JobError::Io);
                                out_eof = true;
                            }
                        }
                    }
                    if !err_eof {
                        match read_pipe(&mut err, &mut buf) {
                            Ok(None) => err_eof = true,
                            Ok(Some(n)) => {
                                if stdout.len().saturating_add(stderr.len()).saturating_add(n)
                                    > max_output
                                {
                                    error = Some(JobError::OutputLimit);
                                    unsafe {
                                        libc::kill(-pid, libc::SIGKILL);
                                    }
                                } else {
                                    stderr.extend_from_slice(&buf[..n]);
                                }
                            }
                            Err(e)
                                if e.kind() == io::ErrorKind::WouldBlock
                                    || e.kind() == io::ErrorKind::Interrupted => {}
                            Err(_) => {
                                error.get_or_insert(JobError::Io);
                                err_eof = true;
                            }
                        }
                    }
                    let status = match child.try_wait() {
                        Ok(s) => s,
                        Err(_) => {
                            error.get_or_insert(JobError::Io);
                            unsafe {
                                libc::kill(-pid, libc::SIGKILL);
                            }
                            let _ = child.kill();
                            None
                        }
                    };
                    if status.is_some() && exit_at.is_none() {
                        exit_at = Some(Instant::now());
                    }
                    // A descendant may inherit a pipe after the command exits; bound that drain window.
                    if exit_at.is_some_and(|t| {
                        Instant::now().duration_since(t) > Duration::from_millis(150)
                    }) {
                        unsafe {
                            libc::kill(-pid, libc::SIGKILL);
                        }
                        out_eof = true;
                        err_eof = true;
                    }
                    if status.is_some() && out_eof && err_eof {
                        let _ = tx.send(result(
                            generation,
                            status.and_then(|s| s.code()),
                            stdout,
                            stderr,
                            error,
                        ));
                        return;
                    }
                    thread::sleep(Duration::from_millis(5));
                }
            })?;
        Ok(Self {
            generation,
            rx,
            cancel: state,
            dispatch,
            complete: false,
        })
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn poll(&mut self) -> Option<JobResult> {
        if self.complete {
            return None;
        }
        match self.rx.try_recv() {
            Ok(r) => {
                self.complete = true;
                Some(r)
            }
            Err(TryRecvError::Disconnected) => {
                self.complete = true;
                Some(result(
                    self.generation,
                    None,
                    vec![],
                    vec![],
                    Some(JobError::Io),
                ))
            }
            Err(TryRecvError::Empty) => None,
        }
    }
    pub fn cancel(&mut self) {
        if !self.complete {
            self.cancel.store(true, Ordering::Release);
            let _ = self
                .dispatch
                .compare_exchange(0, 2, Ordering::AcqRel, Ordering::Acquire);
        }
    }
    pub fn cancel_and_wait(&mut self, timeout: Duration) -> Option<JobResult> {
        let end = Instant::now() + timeout;
        self.cancel();
        while Instant::now() < end {
            if let Some(r) = self.poll() {
                return Some(r);
            }
            thread::sleep(Duration::from_millis(5));
        }
        None
    }
}
fn read_pipe<R: Read>(r: &mut R, b: &mut [u8]) -> io::Result<Option<usize>> {
    match r.read(b) {
        Ok(0) => Ok(None),
        Ok(n) => Ok(Some(n)),
        Err(e) => Err(e),
    }
}
fn result(
    generation: u64,
    status: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    error: Option<JobError>,
) -> JobResult {
    JobResult {
        generation,
        status,
        stdout,
        stderr,
        error,
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel()
    }
}
