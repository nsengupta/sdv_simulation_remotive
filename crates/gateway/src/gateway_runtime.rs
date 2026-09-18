//! `TwinRuntimeBuilder` — gateway runtime API for assembling the live Digital Twin.
//!
//! One application (`main`) typically owns both the **Digital Twin** (install + ingress via
//! this builder) and the **Dashboard** (observation receivers wired in setup). The builder
//! connects:
//! - `VehicleController` (actor tree)
//! - Diagnostic channel (unbounded) — caller creates, passes sender
//! - Transition channel (bounded) — caller creates, passes sender
//! - CAN reader thread
//! - Ingress dispatch loop
//!
//! The runtime is observation-only: it does not open an actuation command channel or
//! publish actuator CMD frames. `DefaultActuationManager` is a no-op without a sender.
//!
//! # Lifecycle
//! 1. `TwinRuntimeBuilder::new` — minimal defaults
//! 2. `.with_car_identity(...)`, `.with_can_interface(...)`, etc.
//! 3. `.install_controller.await` — spawns actor tree
//! 4. `.spawn_runtime` — spawns CAN reader, returns `JoinHandle`
//! 5. (Gateway) `.run` = `install_controller` + `spawn_runtime` + await dispatch

use anyhow::Result;
use common::DiagnosticRecord;
use common::facade::{
    AssemblyTopology, PublishedTransitionRecord, TwinIngressEvent, VehicleController,
    VehicleControllerRuntimeOptions, VssSignal, spawn_stdout_diagnostic_observer,
};
use socketcan::{CanSocket, Socket};
use std::future::Future;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::ingress;
use crate::transition_log;

/// Default SocketCAN interface. Re-exported from [`vehicle_device_bus`] so every
/// process on the bus shares one transport-level default.
pub use vehicle_device_bus::DEFAULT_CAN_INTERFACE;

/// Assembles and runs the live Digital Twin runtime.
///
/// Gateway creates channels, attaches observers, and calls [`run`](Self::run)
/// (or `install_controller` + `spawn_runtime` when composing capture/tee in `main`).
pub struct TwinRuntimeBuilder {
    car_identity: Option<String>,
    can_interface: String,
    diagnostic_tx: Option<mpsc::UnboundedSender<DiagnosticRecord>>,
    transition_tx: Option<mpsc::Sender<PublishedTransitionRecord>>,
    /// Optional compatibility switch. Defaults to bridge-owned lifecycle.
    auto_power_on: bool,
    /// When true, print the gateway listen banner. Default false so an in-process
    /// Dashboard TTY is not corrupted. Standalone gateway enables this.
    ingress_console_log: bool,
}

impl TwinRuntimeBuilder {
    /// Create a new builder with safe defaults.
    pub fn new() -> Self {
        Self {
            car_identity: None,
            can_interface: DEFAULT_CAN_INTERFACE.to_string(),
            diagnostic_tx: None,
            transition_tx: None,
            auto_power_on: false,
            ingress_console_log: false,
        }
    }

    /// Set the car identity string (e.g. `"My-Opel-Corsa-1.4-GSi"`).
    pub fn with_car_identity(mut self, identity: impl Into<String>) -> Self {
        self.car_identity = Some(identity.into());
        self
    }

    /// Set the SocketCAN interface name (default: [`DEFAULT_CAN_INTERFACE`]).
    pub fn with_can_interface(mut self, iface: impl Into<String>) -> Self {
        self.can_interface = iface.into();
        self
    }

    /// Accepted for CLI compatibility. ACK correlation was removed, so this is a no-op.
    pub fn with_actuation_ingress_trace(self) -> Self {
        self
    }

    /// Provide a sender for diagnostic records. Caller retains the receiver.
    pub fn with_diagnostic_channel(mut self, tx: mpsc::UnboundedSender<DiagnosticRecord>) -> Self {
        self.diagnostic_tx = Some(tx);
        self
    }

    /// Provide a sender for transition records. Caller retains the receiver.
    pub fn with_transition_channel(mut self, tx: mpsc::Sender<PublishedTransitionRecord>) -> Self {
        self.transition_tx = Some(tx);
        self
    }

    /// When `true`, [`Self::spawn_runtime`] sends `PowerOn` after workers start.
    /// Gateway callers leave this disabled because the Remotive bridge owns lifecycle.
    pub fn with_auto_power_on(mut self, enabled: bool) -> Self {
        self.auto_power_on = enabled;
        self
    }

    /// Whether [`Self::spawn_runtime`] will auto-send `PowerOn`.
    pub(crate) fn auto_power_on(&self) -> bool {
        self.auto_power_on
    }

    /// Enable the gateway listen banner (standalone gateway console only).
    pub fn with_ingress_console_log(mut self, enabled: bool) -> Self {
        self.ingress_console_log = enabled;
        self
    }

    /// Whether the listen banner is printed to stdout.
    pub fn ingress_console_log(&self) -> bool {
        self.ingress_console_log
    }

    /// Attach a stdout diagnostic observer for the given receiver.
    /// Convenience: wires the observer and returns the `JoinHandle`.
    pub fn with_stdout_diagnostic_observer(
        &self,
        rx: mpsc::UnboundedReceiver<DiagnosticRecord>,
    ) -> JoinHandle<()> {
        spawn_stdout_diagnostic_observer(rx)
    }

    /// Spawn a transition log task for the given receiver.
    /// Returns the `JoinHandle` so the caller can keep it alive.
    pub fn with_transition_log_task(
        &self,
        rx: mpsc::Receiver<PublishedTransitionRecord>,
        color: bool,
    ) -> JoinHandle<()> {
        transition_log::spawn_transition_log_task(rx, color)
    }

    /// Install the controller actor tree.
    ///
    /// Observation-only: no actuation command channel is created. Must be called
    /// before [`spawn_runtime`](Self::spawn_runtime) when composing capture/tee.
    pub async fn install_controller(
        &mut self,
    ) -> Result<(VehicleController, VehicleControllerRuntimeOptions)> {
        let identity = self
            .car_identity
            .clone()
            .ok_or_else(|| anyhow::anyhow!("car_identity must be set before install_controller"))?;

        let runtime_options = VehicleControllerRuntimeOptions {
            assembly_topology: AssemblyTopology::ObservedEcus,
            actuation_command_tx: None,
            diagnostic_tx: self.diagnostic_tx.clone(),
            transition_tx: self.transition_tx.clone(),
            ..VehicleControllerRuntimeOptions::default()
        };

        let (controller, _join) =
            VehicleController::install_and_start_with_options(identity, runtime_options.clone())
                .await
                .map_err(|e| anyhow::anyhow!("install controller: {e}"))?;

        Ok((controller, runtime_options))
    }

    /// Spawn background workers (CAN reader + ingress dispatch).
    ///
    /// Call [`install_controller`](Self::install_controller) first.
    /// Returns a `JoinHandle` that resolves when the ingress dispatch loop exits.
    pub fn spawn_runtime(
        &mut self,
        controller: VehicleController,
    ) -> Result<JoinHandle<Result<()>>> {
        let can_interface = self.can_interface.clone();
        let auto_power_on = self.auto_power_on();
        let ingress_console_log = self.ingress_console_log;

        let (can_tx, can_rx) = mpsc::unbounded_channel();
        spawn_can_reader_thread(can_interface.clone(), can_tx)?;

        if ingress_console_log {
            println!("⚡ Gateway on {can_interface} — CAN → TwinIngressEvent → VehicleController");
        }

        let c = controller.clone();
        let _auto_power_on_task =
            spawn_auto_power_on_if_enabled(auto_power_on, move || async move {
                if let Err(e) = c.send_power_on().await {
                    eprintln!("[gateway] PowerOn failed: {e:?}");
                }
            });

        let dispatch = tokio::spawn(run_can_ingress_dispatch_loop(controller, can_rx));

        Ok(dispatch)
    }

    /// Convenience: install controller, spawn runtime, and await the dispatch loop.
    ///
    /// Suitable for Gateway (headless). Dashboard calls
    /// [`install_controller`](Self::install_controller) +
    /// [`spawn_runtime`](Self::spawn_runtime) separately.
    pub async fn run(&mut self) -> Result<()> {
        let (controller, _) = self.install_controller().await?;
        let handle = self.spawn_runtime(controller)?;
        match handle.await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(e),
            Err(e) => Err(anyhow::anyhow!("dispatch loop panicked: {e:?}")),
        }
    }
}

impl Default for TwinRuntimeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

fn spawn_auto_power_on_if_enabled<Action, ActionFuture>(
    enabled: bool,
    action: Action,
) -> Option<JoinHandle<()>>
where
    Action: FnOnce() -> ActionFuture + Send + 'static,
    ActionFuture: Future<Output = ()> + Send + 'static,
{
    enabled.then(|| tokio::spawn(action()))
}

/// Dedicated OS thread for the blocking SocketCAN `read_frame` loop.
///
/// `CanSocket::read_frame` waits synchronously until a frame arrives (or errors).
/// A Tokio task would hold a runtime worker for the life of the gateway; this OS
/// thread keeps the async dispatch loop free and forwards frames on `tx`.
fn spawn_can_reader_thread(
    can_interface: String,
    tx: mpsc::UnboundedSender<TwinIngressEvent>,
) -> Result<std::thread::JoinHandle<()>> {
    let socket = CanSocket::open(&can_interface)?;
    let thread_name = format!("gateway-can-reader-{can_interface}");
    let handle = std::thread::Builder::new()
        .name(thread_name)
        .spawn(move || {
            loop {
                let frame = match socket.read_frame() {
                    Ok(frame) => frame,
                    Err(e) => {
                        eprintln!("[gateway-can-reader]: read_frame failed: {e:?}");
                        continue;
                    }
                };
                let Some(twin_ingress) = ingress::can_frame_to_twin_ingress(&frame) else {
                    continue;
                };
                if matches!(
                    twin_ingress,
                    TwinIngressEvent::Telemetry(VssSignal::Speed(_))
                ) {
                    continue;
                }
                if tx.send(twin_ingress).is_err() {
                    break;
                }
            }
        })?;
    Ok(handle)
}

async fn run_can_ingress_dispatch_loop(
    controller: VehicleController,
    mut rx: mpsc::UnboundedReceiver<TwinIngressEvent>,
) -> Result<()> {
    while let Some(twin_ingress) = rx.recv().await {
        controller
            .submit_twin_ingress(twin_ingress)
            .await
            .map_err(|e| anyhow::anyhow!("submit twin ingress: {e:?}"))?;
    }
    Err(anyhow::anyhow!(
        "CAN ingress channel closed: reader thread exited"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::facade::AssemblyTopology;
    use std::sync::Arc;

    #[tokio::test]
    async fn builder_defaults_do_not_panic() {
        let builder = TwinRuntimeBuilder::new();
        assert!(builder.car_identity.is_none());
        assert_eq!(builder.can_interface, DEFAULT_CAN_INTERFACE);
        assert_eq!(
            DEFAULT_CAN_INTERFACE,
            vehicle_device_bus::DEFAULT_CAN_INTERFACE
        );
    }

    #[tokio::test]
    async fn install_controller_is_observation_only() {
        let mut builder = TwinRuntimeBuilder::new().with_car_identity("observation-only");
        let (_controller, opts) = builder
            .install_controller()
            .await
            .expect("install controller");
        assert_eq!(opts.assembly_topology, AssemblyTopology::ObservedEcus);
        assert!(
            opts.actuation_command_tx.is_none(),
            "gateway must not install an actuation command channel"
        );
    }

    #[tokio::test]
    async fn builder_accepts_channels() {
        let (diag_tx, _diag_rx) = mpsc::unbounded_channel();
        let (trans_tx, _trans_rx) = mpsc::channel(256);

        let builder = TwinRuntimeBuilder::new()
            .with_car_identity("test-car")
            .with_diagnostic_channel(diag_tx)
            .with_transition_channel(trans_tx);

        assert!(builder.diagnostic_tx.is_some());
        assert!(builder.transition_tx.is_some());
    }

    #[tokio::test]
    async fn builder_defaults_to_bridge_owned_lifecycle() {
        let builder = TwinRuntimeBuilder::new();
        assert!(!builder.auto_power_on());
    }

    #[tokio::test]
    async fn bridge_owned_lifecycle_runtime_never_schedules_gateway_power_on() {
        let builder = TwinRuntimeBuilder::new();
        let invoked = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let invoked_by_action = Arc::clone(&invoked);

        let task = spawn_auto_power_on_if_enabled(builder.auto_power_on(), move || async move {
            invoked_by_action.store(true, std::sync::atomic::Ordering::SeqCst);
        });

        assert!(task.is_none());
        tokio::task::yield_now().await;
        assert!(!invoked.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[tokio::test]
    async fn builder_with_auto_power_on_false() {
        let builder = TwinRuntimeBuilder::new().with_auto_power_on(false);
        assert!(!builder.auto_power_on());
    }

    #[tokio::test]
    async fn ingress_console_log_defaults_false() {
        let builder = TwinRuntimeBuilder::new();
        assert!(!builder.ingress_console_log());
    }

    #[tokio::test]
    async fn ingress_console_log_can_be_enabled() {
        let builder = TwinRuntimeBuilder::new().with_ingress_console_log(true);
        assert!(builder.ingress_console_log());
    }

    #[tokio::test]
    async fn builder_install_controller_needs_identity() {
        let mut builder = TwinRuntimeBuilder::new();
        let result = builder.install_controller().await;
        assert!(result.is_err());
        assert!(
            format!("{:?}", result).contains("car_identity"),
            "expected error about missing car_identity, got: {result:?}"
        );
    }
}
