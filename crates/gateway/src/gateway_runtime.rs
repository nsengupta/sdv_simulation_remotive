//! `TwinRuntimeBuilder` — gateway runtime API for assembling the live Digital Twin.
//!
//! One application (`main`) typically owns both the **Digital Twin** (install + ingress via
//! this builder) and the **Dashboard** (observation receivers wired in setup). The builder
//! connects:
//! - `VehicleController` (actor tree)
//! - Diagnostic channel (unbounded) — caller creates, passes sender
//! - Transition channel (bounded) — caller creates, passes sender
//! - Actuation channel — created internally (CAN egress detail)
//! - CAN reader thread
//! - Actuation command publishers
//! - Ingress dispatch loop
//!
//! # Lifecycle
//! 1. `TwinRuntimeBuilder::new` — minimal defaults
//! 2. `.with_car_identity(...)`, `.with_can_interface(...)`, etc.
//! 3. `.install_controller.await` — spawns actor tree, creates actuation channel
//! 4. `.spawn_runtime` — spawns CAN reader, publishers, returns `JoinHandle`
//! 5. (Gateway) `.run` = `install_controller` + `spawn_runtime` + await dispatch

use anyhow::Result;
use common::DiagnosticRecord;
use common::facade::{
    ActuationCommand, PublishedTransitionRecord, TwinIngressEvent, VehicleController,
    VehicleControllerRuntimeOptions, VssSignal, spawn_stdout_diagnostic_observer,
};
use socketcan::{CanSocket, Socket};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use vehicle_device_bus::devices::front_headlamp::can::{
    decode_payload_from_can_frame, encode_command_frame,
};
use vehicle_device_bus::devices::front_headlamp::policy::{
    FrontHeadlampPolicy, FrontHeadlampPolicyDecision,
};
use vehicle_device_bus::devices::wiper::can::encode_wiper_command_frame;

use crate::ingress;
use crate::transition_log;

/// Default SocketCAN interface (matches emulator and front_headlamp_actuator).
pub const DEFAULT_CAN_INTERFACE: &str = "vcan0";

const ACTUATION_COMMAND_CHANNEL_CAPACITY: usize = 64;
/// Bound on the off-task ingress-log queue. Logging is best-effort: a frozen console
/// (Ctrl-S / XOFF) must not stall the CAN ingress dispatch loop (which also delivers ACKs to the
/// twin), so lines are dropped once this fills.
const INGRESS_LOG_CHANNEL_CAPACITY: usize = 512;

/// Selects how controller actuation commands leave the gateway.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ActuationEgressMode {
    /// Phase I behavior: consume commands without opening actuator CAN publisher sockets.
    #[default]
    Null,
    /// Backward-compatible front-headlamp and wiper CAN publishers.
    LegacyCan,
}

/// Messages forwarded from the dedicated CAN reader thread into the async dispatch loop.
enum CanIngressEnvelope {
    TwinIngress(TwinIngressEvent),
    ActuationResponse {
        twin_ingress: TwinIngressEvent,
        session: u16,
        sequence: u32,
    },
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Assembles and runs the live Digital Twin runtime.
///
/// Gateway creates channels, attaches observers, and calls [`run`](Self::run)
/// (or `install_controller` + `spawn_runtime` when composing capture/tee in `main`).
pub struct TwinRuntimeBuilder {
    car_identity: Option<String>,
    can_interface: String,
    trace_actuation_ingress: bool,
    diagnostic_tx: Option<mpsc::UnboundedSender<DiagnosticRecord>>,
    transition_tx: Option<mpsc::Sender<PublishedTransitionRecord>>,
    headlamp_policy: Arc<Mutex<FrontHeadlampPolicy>>,
    /// Optional compatibility switch. Phase I defaults to bridge-owned lifecycle.
    auto_power_on: bool,
    /// When true, format headlamp ACK/NACK ingress lines to stdout. Default false so an
    /// in-process Dashboard TTY is not corrupted. Standalone gateway enables this.
    ingress_console_log: bool,
    /// Created internally by [`install_controller`]; consumed by [`spawn_runtime`].
    actuation_cmd_rx: Option<mpsc::Receiver<ActuationCommand>>,
    actuation_egress_mode: ActuationEgressMode,
}

impl TwinRuntimeBuilder {
    /// Create a new builder with safe defaults.
    pub fn new() -> Self {
        Self {
            car_identity: None,
            can_interface: DEFAULT_CAN_INTERFACE.to_string(),
            trace_actuation_ingress: false,
            diagnostic_tx: None,
            transition_tx: None,
            headlamp_policy: Arc::new(Mutex::new(FrontHeadlampPolicy::default())),
            auto_power_on: false,
            ingress_console_log: false,
            actuation_cmd_rx: None,
            actuation_egress_mode: ActuationEgressMode::default(),
        }
    }

    /// Set the car identity string (e.g. `"My-Opel-Corsa-1.4-GSi"`).
    pub fn with_car_identity(mut self, identity: impl Into<String>) -> Self {
        self.car_identity = Some(identity.into());
        self
    }

    /// Set the SocketCAN interface name (default: `vcan0`).
    pub fn with_can_interface(mut self, iface: impl Into<String>) -> Self {
        self.can_interface = iface.into();
        self
    }

    /// Log ignored CAN ingress frames (headlamp echoes).
    pub fn with_actuation_ingress_trace(mut self) -> Self {
        self.trace_actuation_ingress = true;
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
    /// Phase I Gateway callers leave this disabled because the Remotive bridge owns lifecycle.
    pub fn with_auto_power_on(mut self, enabled: bool) -> Self {
        self.auto_power_on = enabled;
        self
    }

    /// Whether [`Self::spawn_runtime`] will auto-send `PowerOn`.
    pub(crate) fn auto_power_on(&self) -> bool {
        self.auto_power_on
    }

    /// Enable formatted headlamp ACK/NACK `println!` lines (standalone gateway console only).
    pub fn with_ingress_console_log(mut self, enabled: bool) -> Self {
        self.ingress_console_log = enabled;
        self
    }

    /// Whether ingress ACK lines are printed to stdout.
    pub fn ingress_console_log(&self) -> bool {
        self.ingress_console_log
    }

    /// Select actuation egress. Legacy CAN publishing is opt-in.
    pub fn with_actuation_egress_mode(mut self, mode: ActuationEgressMode) -> Self {
        self.actuation_egress_mode = mode;
        self
    }

    /// Return the configured actuation egress mode.
    pub fn actuation_egress_mode(&self) -> ActuationEgressMode {
        self.actuation_egress_mode
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
    /// This creates the actuation channel internally (a CAN-egress implementation detail)
    /// and spawns the `VehicleController` with all configured channels.
    ///
    /// Must be called before [`spawn_runtime`](Self::spawn_runtime).
    pub async fn install_controller(
        &mut self,
    ) -> Result<(VehicleController, VehicleControllerRuntimeOptions)> {
        let identity = self
            .car_identity
            .clone()
            .ok_or_else(|| anyhow::anyhow!("car_identity must be set before install_controller"))?;

        let (actuation_cmd_tx, actuation_cmd_rx) =
            mpsc::channel(ACTUATION_COMMAND_CHANNEL_CAPACITY);

        let runtime_options = VehicleControllerRuntimeOptions {
            actuation_command_tx: Some(actuation_cmd_tx),
            diagnostic_tx: self.diagnostic_tx.clone(),
            transition_tx: self.transition_tx.clone(),
            ..VehicleControllerRuntimeOptions::default()
        };

        let (controller, _join) =
            VehicleController::install_and_start_with_options(identity, runtime_options.clone())
                .await
                .map_err(|e| anyhow::anyhow!("install controller: {e}"))?;

        // Store actuation receiver so spawn_runtime can find it.
        self.actuation_cmd_rx = Some(actuation_cmd_rx);

        Ok((controller, runtime_options))
    }

    /// Spawn background workers (timer tick, CAN reader, actuation publishers).
    ///
    /// Call [`install_controller`](Self::install_controller) first.
    /// Returns a `JoinHandle` that resolves when the ingress dispatch loop exits.
    pub fn spawn_runtime(
        &mut self,
        controller: VehicleController,
    ) -> Result<JoinHandle<Result<()>>> {
        let can_interface = self.can_interface.clone();
        let headlamp_policy = self.headlamp_policy.clone();
        let trace_actuation_ingress = self.trace_actuation_ingress;
        let auto_power_on = self.auto_power_on();
        let ingress_console_log = self.ingress_console_log;
        let actuation_egress_mode = self.actuation_egress_mode;
        let actuation_cmd_rx = self.actuation_cmd_rx.take().ok_or_else(|| {
            anyhow::anyhow!("install_controller must be called before spawn_runtime")
        })?;

        // Off-hot-path ingress logger (opt-in): never println into a Dashboard-owned TTY.
        let ingress_log_tx = if ingress_console_log {
            let (tx, mut rx) = mpsc::channel::<String>(INGRESS_LOG_CHANNEL_CAPACITY);
            tokio::spawn(async move {
                while let Some(line) = rx.recv().await {
                    println!("{line}");
                }
            });
            Some(tx)
        } else {
            None
        };

        if let ActuationEgressTask::NullDrain(task) = spawn_actuation_egress(
            actuation_egress_mode,
            actuation_cmd_rx,
            can_interface.clone(),
            headlamp_policy.clone(),
            spawn_actuation_command_publishers,
        ) {
            tokio::spawn(async move {
                if let Err(error) = task.await {
                    eprintln!("[gateway] null actuation drain failed: {error}");
                }
            });
        }

        // CAN reader thread (blocking I/O)
        let (can_tx, can_rx) = mpsc::unbounded_channel();
        spawn_can_reader_thread(
            can_interface.clone(),
            headlamp_policy,
            trace_actuation_ingress,
            can_tx,
        )?;

        if ingress_console_log {
            println!("⚡ Gateway on {can_interface} — CAN → TwinIngressEvent → VehicleController");
            match actuation_egress_mode {
                ActuationEgressMode::Null => println!("[gateway] actuation egress: null drain"),
                ActuationEgressMode::LegacyCan => println!(
                    "[gateway] front-headlamp + wiper CMD egress on CAN; \
                     run `cargo run -p front_headlamp_actuator` and `cargo run -p wiper_actuator`"
                ),
            }
        }

        if auto_power_on {
            let c = controller.clone();
            tokio::spawn(async move {
                if let Err(e) = c.send_power_on().await {
                    eprintln!("[gateway] PowerOn failed: {e:?}");
                }
            });
        }

        // Spawn ingress dispatch loop and return its JoinHandle.
        let dispatch = tokio::spawn(run_can_ingress_dispatch_loop(
            controller,
            can_rx,
            ingress_log_tx,
        ));

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

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

enum ActuationEgressTask {
    NullDrain(JoinHandle<()>),
    LegacyCan,
}

fn spawn_actuation_egress(
    mode: ActuationEgressMode,
    actuation_cmd_rx: mpsc::Receiver<ActuationCommand>,
    can_interface: String,
    front_headlamp_policy: Arc<Mutex<FrontHeadlampPolicy>>,
    spawn_legacy_can: impl FnOnce(
        mpsc::Receiver<ActuationCommand>,
        String,
        Arc<Mutex<FrontHeadlampPolicy>>,
    ),
) -> ActuationEgressTask {
    match mode {
        ActuationEgressMode::Null => {
            ActuationEgressTask::NullDrain(tokio::spawn(drain_actuation_commands(actuation_cmd_rx)))
        }
        ActuationEgressMode::LegacyCan => {
            spawn_legacy_can(actuation_cmd_rx, can_interface, front_headlamp_policy);
            ActuationEgressTask::LegacyCan
        }
    }
}

/// Consume commands until every sender closes.
async fn drain_actuation_commands(mut actuation_cmd_rx: mpsc::Receiver<ActuationCommand>) {
    while actuation_cmd_rx.recv().await.is_some() {}
}

/// Dedicated OS thread for blocking `read_frame` loop.
fn spawn_can_reader_thread(
    can_interface: String,
    front_headlamp_policy: Arc<Mutex<FrontHeadlampPolicy>>,
    trace_actuation_ingress: bool,
    tx: mpsc::UnboundedSender<CanIngressEnvelope>,
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
                if let Some(twin_ingress) = ingress::can_frame_to_twin_ingress(&frame) {
                    if matches!(
                        twin_ingress,
                        TwinIngressEvent::Telemetry(VssSignal::Speed(_))
                    ) {
                        continue;
                    }
                    if tx
                        .send(CanIngressEnvelope::TwinIngress(twin_ingress))
                        .is_err()
                    {
                        break;
                    }
                    continue;
                }
                if let Some(payload) = decode_payload_from_can_frame(&frame) {
                    let decision = {
                        let mut policy = front_headlamp_policy
                            .lock()
                            .expect("front-headlamp policy lock");
                        policy.on_response(payload)
                    };
                    match decision {
                        FrontHeadlampPolicyDecision::Accept {
                            twin_ingress,
                            session,
                            sequence,
                        } => {
                            if tx
                                .send(CanIngressEnvelope::ActuationResponse {
                                    twin_ingress,
                                    session,
                                    sequence,
                                })
                                .is_err()
                            {
                                break;
                            }
                        }
                        FrontHeadlampPolicyDecision::Ignore(reason) => {
                            if trace_actuation_ingress {
                                eprintln!(
                                    "[actuation-can-ingress trace ignored]: reason={reason} session={} seq={}",
                                    payload.session_id, payload.sequence_no
                                );
                            }
                        }
                    }
                }
            }
        })?;
    Ok(handle)
}

/// Fan-out: one actuation channel -> headlamp publisher + wiper publisher.
fn spawn_actuation_command_publishers(
    mut actuation_cmd_rx: mpsc::Receiver<ActuationCommand>,
    can_interface: String,
    front_headlamp_policy: Arc<Mutex<FrontHeadlampPolicy>>,
) {
    let (headlamp_tx, headlamp_rx) = mpsc::channel(ACTUATION_COMMAND_CHANNEL_CAPACITY);
    let (wiper_tx, wiper_rx) = mpsc::channel(ACTUATION_COMMAND_CHANNEL_CAPACITY);

    tokio::spawn(async move {
        while let Some(cmd) = actuation_cmd_rx.recv().await {
            let tx = match &cmd {
                ActuationCommand::StartWiper | ActuationCommand::StopWiper => &wiper_tx,
                ActuationCommand::SwitchFrontHeadlampOn { .. }
                | ActuationCommand::SwitchFrontHeadlampOff { .. } => &headlamp_tx,
                ActuationCommand::SetTurnLights { .. } => continue,
            };
            if tx.send(cmd).await.is_err() {
                break;
            }
        }
    });

    spawn_front_headlamp_command_publisher(
        headlamp_rx,
        can_interface.clone(),
        front_headlamp_policy,
    );
    spawn_wiper_command_publisher(wiper_rx, can_interface);
}

/// Egress: twin actuation intent -> policy pending state -> CMD frame on CAN.
fn spawn_front_headlamp_command_publisher(
    mut actuation_cmd_rx: mpsc::Receiver<ActuationCommand>,
    can_interface: String,
    front_headlamp_policy: Arc<Mutex<FrontHeadlampPolicy>>,
) {
    tokio::spawn(async move {
        let socket = match CanSocket::open(&can_interface) {
            Ok(s) => s,
            Err(e) => {
                eprintln!(
                    "[gateway]: cannot open CAN {can_interface} for front-headlamp CMD TX: {e}"
                );
                return;
            }
        };
        while let Some(cmd) = actuation_cmd_rx.recv().await {
            {
                let mut policy = front_headlamp_policy
                    .lock()
                    .expect("front-headlamp policy lock");
                policy.on_command_sent(&cmd);
            }
            match encode_command_frame(&cmd) {
                Ok(frame) => {
                    if let Err(e) = socket.write_frame(&frame) {
                        eprintln!("[gateway]: front-headlamp CMD write_frame failed: {e:?}");
                    }
                }
                Err(e) => eprintln!("[gateway]: encode front-headlamp CMD failed: {e:?}"),
            }
        }
    });
}

/// Egress: twin wiper actuation intent -> CMD frame on CAN (fire-and-forget).
fn spawn_wiper_command_publisher(
    mut actuation_cmd_rx: mpsc::Receiver<ActuationCommand>,
    can_interface: String,
) {
    tokio::spawn(async move {
        let socket = match CanSocket::open(&can_interface) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[gateway]: cannot open CAN {can_interface} for wiper CMD TX: {e}");
                return;
            }
        };
        while let Some(cmd) = actuation_cmd_rx.recv().await {
            if !matches!(
                cmd,
                ActuationCommand::StartWiper | ActuationCommand::StopWiper
            ) {
                continue;
            }
            match encode_wiper_command_frame(&cmd) {
                Ok(frame) => {
                    if let Err(e) = socket.write_frame(&frame) {
                        eprintln!("[gateway]: wiper CMD write_frame failed: {e:?}");
                    }
                }
                Err(e) => eprintln!("[gateway]: wiper CMD encode failed: {e}"),
            }
        }
    });
}

/// Format the wire-level ingress line for a headlamp ACK/NACK, or `None` for other events.
fn format_front_headlamp_ingress(
    session: u16,
    sequence: u32,
    twin_ingress: &TwinIngressEvent,
) -> Option<String> {
    let (icon, msg) = match twin_ingress {
        TwinIngressEvent::FrontHeadlampCommandConfirmed { on_command: true } => ("✓", "ACK_ON"),
        TwinIngressEvent::FrontHeadlampCommandConfirmed { on_command: false } => ("✓", "ACK_OFF"),
        TwinIngressEvent::FrontHeadlampCommandRejected { on_command: true } => ("✗", "NACK_ON"),
        TwinIngressEvent::FrontHeadlampCommandRejected { on_command: false } => ("✗", "NACK_OFF"),
        _ => return None,
    };
    Some(format!(
        "[actuation-can-ingress session={session} seq={sequence}]: {icon} {msg}"
    ))
}

async fn run_can_ingress_dispatch_loop(
    controller: VehicleController,
    mut rx: mpsc::UnboundedReceiver<CanIngressEnvelope>,
    ingress_log_tx: Option<mpsc::Sender<String>>,
) -> Result<()> {
    while let Some(msg) = rx.recv().await {
        match msg {
            CanIngressEnvelope::TwinIngress(twin_ingress) => {
                controller
                    .submit_twin_ingress(twin_ingress)
                    .await
                    .map_err(|e| anyhow::anyhow!("submit twin ingress: {e:?}"))?;
            }
            CanIngressEnvelope::ActuationResponse {
                twin_ingress,
                session,
                sequence,
            } => {
                let line = format_front_headlamp_ingress(session, sequence, &twin_ingress);
                controller
                    .submit_twin_ingress(twin_ingress)
                    .await
                    .map_err(|e| anyhow::anyhow!("submit twin ingress: {e:?}"))?;
                if let (Some(tx), Some(line)) = (ingress_log_tx.as_ref(), line) {
                    let _ = tx.try_send(line);
                }
            }
        }
    }
    Err(anyhow::anyhow!(
        "CAN ingress channel closed: reader thread exited"
    ))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use common::facade::CorrelationId;

    #[tokio::test]
    async fn builder_defaults_do_not_panic() {
        let builder = TwinRuntimeBuilder::new();
        assert!(builder.car_identity.is_none());
        assert_eq!(builder.can_interface, DEFAULT_CAN_INTERFACE);
    }

    #[test]
    fn builder_defaults_to_null_actuation_egress() {
        let builder = TwinRuntimeBuilder::new();
        assert_eq!(builder.actuation_egress_mode(), ActuationEgressMode::Null);
    }

    #[test]
    fn builder_can_explicitly_select_legacy_can_actuation_egress() {
        let builder =
            TwinRuntimeBuilder::new().with_actuation_egress_mode(ActuationEgressMode::LegacyCan);
        assert_eq!(
            builder.actuation_egress_mode(),
            ActuationEgressMode::LegacyCan
        );
    }

    #[tokio::test]
    async fn runtime_selector_uses_only_null_drain_and_completes_at_channel_closure() {
        let (tx, rx) = mpsc::channel(4);
        let task = match spawn_actuation_egress(
            ActuationEgressMode::Null,
            rx,
            "must-not-open-can".to_string(),
            Arc::new(Mutex::new(FrontHeadlampPolicy::default())),
            |_, _, _| panic!("Null mode invoked legacy CAN publisher spawning"),
        ) {
            ActuationEgressTask::NullDrain(task) => task,
            ActuationEgressTask::LegacyCan => {
                panic!("Null mode selected legacy CAN publishers")
            }
        };

        tx.send(ActuationCommand::StartWiper)
            .await
            .expect("send wiper command");
        tx.send(ActuationCommand::SetTurnLights {
            correlation_id: CorrelationId {
                source_id: "gateway-null-test".into(),
                session_id: 9,
                sequence_no: 1,
            },
            left_on: true,
            right_on: true,
        })
        .await
        .expect("send turn-light command");
        drop(tx);

        let completion: () = tokio::time::timeout(std::time::Duration::from_millis(250), task)
            .await
            .expect("null drain did not finish when all senders closed")
            .expect("null drain task panicked");
        assert_eq!(completion, ());
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
