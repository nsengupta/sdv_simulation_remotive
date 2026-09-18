//! Gateway-owned clean exit for the twin host (**not** Dashboard, **not** FSM states).
//!
//! PowerOn/PowerOff stay on CAN (emulator). Dashboard never learns whether the twin is up —
//! it only sees observation connect/disconnect. When the operator stops **Gateway** (e.g.
//! Ctrl+C), this helper is the place to: optional `PowerOff` → wait `Off` → stop actors /
//! CAN ingress → finish the observation tee. Stub until that signal path is wired; see
//! `docs/TODO-twin-lifecycle.md` (TL-6+).

use std::time::Duration;

use anyhow::Result;
use common::facade::VehicleController;

/// Gateway process exit: stop the twin cleanly before the process dies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShutdownCoordinator {
    stop_timeout: Duration,
}

impl ShutdownCoordinator {
    pub fn new(stop_timeout: Duration) -> Self {
        Self { stop_timeout }
    }

    /// Stub: returns `Ok` until Ctrl+C (or equivalent) calls this from Gateway `main`.
    /// Intended: `send_power_off` if needed, await `FsmState::Off` (or timeout), then tear
    /// down actor + ingress. Dashboard stays out of this path.
    pub async fn ensure_stopped_before_exit(&self, _controller: &VehicleController) -> Result<()> {
        let _ = self.stop_timeout;
        Ok(())
    }
}

impl Default for ShutdownCoordinator {
    fn default() -> Self {
        Self::new(Duration::from_secs(30))
    }
}
