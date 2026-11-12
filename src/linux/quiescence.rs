//! Fail-closed, bounded shutdown barrier for a leased panel worker.

use std::time::Duration;

pub const KEY_RELEASE_TIMEOUT: Duration = Duration::from_millis(250);

pub trait Actuator {
    fn revoke_authority(&mut self);
    fn cancel_effects_nonblocking(&mut self);
    fn release_keys_and_sync(&mut self, timeout: Duration) -> Result<(), String>;
    fn fresh_panel_off(&mut self) -> Result<(), String>;
    fn blackout_existing_framebuffer(&mut self) -> Result<(), String>;
}

/// Evidence that keys were released and a fresh panel-off command succeeded.
/// Construction is intentionally restricted to [`attempt`].
#[derive(Debug, PartialEq, Eq)]
pub struct QuiescenceToken {
    transaction: u64,
}

impl QuiescenceToken {
    pub fn transaction(&self) -> u64 {
        self.transaction
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct RecoveryRequired {
    pub transaction: u64,
    /// Number of top-level quiescence transactions attempted.
    pub attempts: u8,
    pub key_release_error: Option<String>,
    pub initial_panel_off_error: Option<String>,
    pub blackout_error: Option<String>,
    pub retry_panel_off_error: Option<String>,
}

/// Run one bounded shutdown transaction. A framebuffer blackout is permitted
/// only as fallback after the initial fresh panel-off command fails.
pub fn attempt<A: Actuator>(
    actuator: &mut A,
    transaction: u64,
) -> Result<QuiescenceToken, RecoveryRequired> {
    actuator.revoke_authority();
    actuator.cancel_effects_nonblocking();
    let keys = actuator.release_keys_and_sync(KEY_RELEASE_TIMEOUT);
    let initial_off = actuator.fresh_panel_off();

    let mut blackout_error = None;
    let mut retry_off_error = None;
    let fallback_off_succeeded;
    if initial_off.is_err() {
        match actuator.blackout_existing_framebuffer() {
            Ok(()) => match actuator.fresh_panel_off() {
                Ok(()) => fallback_off_succeeded = true,
                Err(error) => {
                    retry_off_error = Some(error);
                    fallback_off_succeeded = false;
                }
            },
            Err(error) => {
                blackout_error = Some(error);
                fallback_off_succeeded = false;
            }
        }
    } else {
        fallback_off_succeeded = false;
    }

    if keys.is_ok() && (initial_off.is_ok() || fallback_off_succeeded) {
        return Ok(QuiescenceToken { transaction });
    }
    Err(RecoveryRequired {
        transaction,
        attempts: 1,
        key_release_error: keys.err(),
        initial_panel_off_error: initial_off.err(),
        blackout_error,
        retry_panel_off_error: retry_off_error,
    })
}
