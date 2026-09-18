use super::virtual_car_actor::{VirtualCarActor, VirtualCarActorArgs};
use crate::digital_twin::{CarSnapshot, TwinMessage};
use crate::fsm::FsmEvent;
use crate::observation_records::transition::PublishedTransitionRecord;
use crate::twin_runtime::connectors::{IngressToFsmProjector, Projector};
use crate::twin_runtime::controller::actuation_contract::ActuationCommand;
use crate::{LifecycleCommand, TwinIngressEvent};
use ractor::rpc::CallResult;
use ractor::{ActorRef, MessagingErr, SpawnErr};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct VehicleController {
    actor: ActorRef<TwinMessage>,
    projector: IngressToFsmProjector,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VehicleControllerError {
    Projection(String),
    Messaging(String),
    Timeout,
    ReplyDropped,
}

/// Selects the actor topology installed by the controller.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AssemblyTopology {
    /// Active integration topology: SCCM, BCM, and FLCM are spawned and routed.
    #[default]
    ObservedEcus,
    /// Explicit compatibility topology for legacy headlamp/wiper applications and tests.
    Legacy,
}

#[derive(Debug, Clone)]
pub struct VehicleControllerRuntimeOptions {
    pub assembly_topology: AssemblyTopology,
    pub log_timer_tick: bool,
    pub actuation_command_tx: Option<tokio::sync::mpsc::Sender<ActuationCommand>>,
    pub diagnostic_tx: Option<
        tokio::sync::mpsc::UnboundedSender<
            crate::observation_records::diagnostic::DiagnosticRecord,
        >,
    >,
    pub transition_tx: Option<tokio::sync::mpsc::Sender<PublishedTransitionRecord>>,
    /// Test-only mute switches, forwarded at child spawn in `VirtualCarActor::pre_start`.
    ///
    /// Production callers leave every flag `false`. Contract tests set a flag so that
    /// twinlet ignores tells: the parent can then inject `ZoneReady` itself, or wait
    /// for tell-back timeout, without a real zone reply racing the scenario.
    #[doc(hidden)]
    pub test_silent_headlamp: bool,
    #[doc(hidden)]
    pub test_silent_wiper: bool,
    #[doc(hidden)]
    pub test_silent_bcm: bool,
    #[doc(hidden)]
    pub test_silent_sccm: bool,
    #[doc(hidden)]
    pub test_silent_flcm: bool,
}

impl Default for VehicleControllerRuntimeOptions {
    fn default() -> Self {
        Self {
            assembly_topology: AssemblyTopology::ObservedEcus,
            log_timer_tick: false,
            actuation_command_tx: None,
            diagnostic_tx: None,
            transition_tx: None,
            test_silent_headlamp: false,
            test_silent_wiper: false,
            test_silent_bcm: false,
            test_silent_sccm: false,
            test_silent_flcm: false,
        }
    }
}

impl VehicleController {
    pub async fn install_and_start(
        identity: String,
    ) -> Result<(Self, ractor::concurrency::JoinHandle<()>), SpawnErr> {
        Self::install_and_start_with_options(identity, VehicleControllerRuntimeOptions::default())
            .await
    }

    pub async fn install_and_start_with_options(
        identity: String,
        runtime_options: VehicleControllerRuntimeOptions,
    ) -> Result<(Self, ractor::concurrency::JoinHandle<()>), SpawnErr> {
        let args = VirtualCarActorArgs {
            identity,
            runtime_options,
        };
        let (actor, handle) = ractor::spawn::<VirtualCarActor>(args).await?;
        Ok((Self::new(actor), handle))
    }

    pub fn new(actor: ActorRef<TwinMessage>) -> Self {
        Self {
            actor,
            projector: IngressToFsmProjector,
        }
    }

    /// Expose the underlying actor reference for direct message access (used in tests).
    pub fn get_actor_ref(&self) -> &ActorRef<TwinMessage> {
        &self.actor
    }

    /// Lifecycle: request primary FSM entry into powered operation through canonical twin ingress.
    pub async fn send_power_on(&self) -> Result<(), VehicleControllerError> {
        self.submit_twin_ingress(TwinIngressEvent::Lifecycle(LifecycleCommand::PowerOn))
            .await
    }

    /// Lifecycle: request primary FSM shutdown to `Off` when legal (`Idle` → `Off` in current rules).
    ///
    /// From non-`Idle` powered states the FSM rejects `PowerOff` (see strategy); the message is still delivered.
    pub async fn send_power_off(&self) -> Result<(), VehicleControllerError> {
        self.submit_twin_ingress(TwinIngressEvent::Lifecycle(LifecycleCommand::PowerOff))
            .await
    }

    /// Dashboard Park: request standstill via RPM update (typically `0` to return toward `Idle`).
    pub async fn send_update_rpm(&self, rpm: u16) -> Result<(), VehicleControllerError> {
        self.submit_fsm_event(FsmEvent::UpdateRpm(rpm)).await
    }

    /// Dashboard Park: zero RPM to move toward standstill / `Idle` before stop.
    pub async fn send_park_to_idle(&self) -> Result<(), VehicleControllerError> {
        self.send_update_rpm(0).await
    }

    /// Public ingress path: canonical external input enters through the FSM projector boundary.
    pub async fn submit_twin_ingress(
        &self,
        event: TwinIngressEvent,
    ) -> Result<(), VehicleControllerError> {
        let msg = self
            .projector
            .project(event)
            .map_err(|e| VehicleControllerError::Projection(format!("{e:?}")))?;
        self.actor
            .send_message(msg)
            .map_err(|e| VehicleControllerError::Messaging(format!("{e}")))?;
        Ok(())
    }

    /// Internal/testing bypass for already-derived FSM events.
    #[allow(dead_code)]
    pub(crate) async fn submit_fsm_event(
        &self,
        event: FsmEvent,
    ) -> Result<(), VehicleControllerError> {
        self.actor
            .send_message(event.into())
            .map_err(|e| VehicleControllerError::Messaging(format!("{e}")))?;
        Ok(())
    }

    /// Read-only snapshot API for external observers. The returned [`CarSnapshot`] is stamped with
    /// `as_of_seq` — the ledger sequence of the last FSM event it reflects (Q3 / WI-4) — so callers
    /// can reason about staleness and reconcile against the transition ledger.
    pub async fn get_snapshot(
        &self,
        timeout: Option<Duration>,
    ) -> Result<CarSnapshot, VehicleControllerError> {
        let result: Result<CallResult<CarSnapshot>, MessagingErr<TwinMessage>> = self
            .actor
            .call(|port| TwinMessage::GetStatus(port), timeout)
            .await;

        match result.map_err(|e| VehicleControllerError::Messaging(format!("{e}")))? {
            CallResult::Success(snapshot) => Ok(snapshot),
            CallResult::Timeout => Err(VehicleControllerError::Timeout),
            CallResult::SenderError => Err(VehicleControllerError::ReplyDropped),
        }
    }
}
