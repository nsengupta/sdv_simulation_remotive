use anyhow::{Result, anyhow};
use async_trait::async_trait;
use common::{LifecycleCommand, ObservedEcuSignal, VssSignal};
use emulator::sink::FrameSink;
use remotive_bridge::session::{SessionConfig, run_connected_session, run_session};
use remotive_bridge::source::{
    BrokerObservation, ObservationConnector, ObservationSource, RpmSource, ShutdownSource,
};
use socketcan::CanFrame;
use std::collections::VecDeque;
use std::future::pending;
use std::num::NonZeroUsize;

#[derive(Default)]
struct RecordingSink {
    frames: Vec<CanFrame>,
    attempts: usize,
    fail_at: Option<usize>,
}

impl FrameSink for RecordingSink {
    fn write_frame(&mut self, frame: CanFrame) -> Result<()> {
        let attempt = self.attempts;
        self.attempts += 1;
        if self.fail_at == Some(attempt) {
            return Err(anyhow!("injected sink failure"));
        }
        self.frames.push(frame);
        Ok(())
    }
}

struct FakeObservations(VecDeque<Result<BrokerObservation>>);

#[async_trait]
impl ObservationSource for FakeObservations {
    async fn next_observation(&mut self) -> Result<BrokerObservation> {
        match self.0.pop_front() {
            Some(item) => item,
            None => pending().await,
        }
    }
}

struct RepeatingObservations {
    remaining: usize,
}

#[async_trait]
impl ObservationSource for RepeatingObservations {
    async fn next_observation(&mut self) -> Result<BrokerObservation> {
        if self.remaining == 0 {
            pending().await
        } else {
            self.remaining -= 1;
            Ok(BrokerObservation::HazardButton(true))
        }
    }
}

struct FakeRpm(VecDeque<u16>);

#[async_trait]
impl RpmSource for FakeRpm {
    async fn next_rpm(&mut self) -> Result<u16> {
        match self.0.pop_front() {
            Some(rpm) => Ok(rpm),
            None => pending().await,
        }
    }
}

struct PendingShutdown;

#[async_trait]
impl ShutdownSource for PendingShutdown {
    async fn wait(&mut self) -> Result<()> {
        pending().await
    }
}

struct ImmediateShutdown;

#[async_trait]
impl ShutdownSource for ImmediateShutdown {
    async fn wait(&mut self) -> Result<()> {
        Ok(())
    }
}

struct FailedConnector;

#[async_trait]
impl ObservationConnector for FailedConnector {
    type Source = FakeObservations;

    async fn connect(self) -> Result<Self::Source> {
        Err(anyhow!("subscription rejected"))
    }
}

fn assert_controlled_edges(frames: &[CanFrame]) {
    assert_eq!(
        LifecycleCommand::from_can_frame(frames.first().unwrap()),
        Some(LifecycleCommand::PowerOn)
    );
    assert_eq!(
        VssSignal::from_can_frame(&frames[frames.len() - 2]),
        Some(VssSignal::EngineRpm(0))
    );
    assert_eq!(
        LifecycleCommand::from_can_frame(frames.last().unwrap()),
        Some(LifecycleCommand::PowerOff)
    );
    assert_eq!(
        frames
            .iter()
            .filter(
                |frame| LifecycleCommand::from_can_frame(frame) == Some(LifecycleCommand::PowerOn)
            )
            .count(),
        1
    );
    assert_eq!(
        frames
            .iter()
            .filter(
                |frame| LifecycleCommand::from_can_frame(frame) == Some(LifecycleCommand::PowerOff)
            )
            .count(),
        1
    );
}

#[tokio::test]
async fn readings_limit_orders_power_on_body_and_controlled_trailer() {
    let mut sink = RecordingSink::default();
    let mut observations =
        FakeObservations(VecDeque::from([Ok(BrokerObservation::HazardButton(true))]));
    let mut rpm = FakeRpm(VecDeque::from([1100, 1200]));
    let mut shutdown = PendingShutdown;

    run_session(
        &mut sink,
        &mut observations,
        &mut rpm,
        &mut shutdown,
        SessionConfig {
            max_readings: NonZeroUsize::new(2),
        },
    )
    .await
    .unwrap();

    assert_controlled_edges(&sink.frames);
    assert!(sink.frames[1..sink.frames.len() - 2].iter().any(|frame| {
        ObservedEcuSignal::from_can_frame(frame) == Some(ObservedEcuSignal::HazardButton(true))
    }));
    assert_eq!(
        sink.frames
            .iter()
            .filter(|frame| matches!(VssSignal::from_can_frame(frame), Some(VssSignal::EngineRpm(rpm)) if rpm != 0))
            .count(),
        2
    );
    assert!(sink.frames.iter().all(|frame| {
        !matches!(
            VssSignal::from_can_frame(frame),
            Some(VssSignal::AmbientLux(_) | VssSignal::RainDetected(_) | VssSignal::Speed(_))
        )
    }));
}

#[tokio::test]
async fn sustained_ready_hazards_cannot_run_ahead_of_ready_rpm_until_exhaustion() {
    let mut sink = RecordingSink::default();
    let mut observations = RepeatingObservations { remaining: 64 };
    let mut rpm = FakeRpm(VecDeque::from([1200]));
    let mut shutdown = PendingShutdown;

    run_session(
        &mut sink,
        &mut observations,
        &mut rpm,
        &mut shutdown,
        SessionConfig {
            max_readings: NonZeroUsize::new(1),
        },
    )
    .await
    .unwrap();

    let rpm_index = sink
        .frames
        .iter()
        .position(|frame| VssSignal::from_can_frame(frame) == Some(VssSignal::EngineRpm(1200)))
        .expect("ready RPM must be emitted");
    let hazard_count_before_rpm = sink.frames[..rpm_index]
        .iter()
        .filter(|frame| {
            ObservedEcuSignal::from_can_frame(frame) == Some(ObservedEcuSignal::HazardButton(true))
        })
        .count();

    assert!(
        hazard_count_before_rpm <= 1,
        "fair scheduler allowed more than one queued hazard ahead of ready RPM"
    );
    assert_controlled_edges(&sink.frames);
}

#[tokio::test]
async fn sustained_ready_hazards_cannot_starve_ready_shutdown() {
    let mut sink = RecordingSink::default();
    let mut observations = RepeatingObservations { remaining: 64 };
    let mut rpm = FakeRpm(VecDeque::new());
    let mut shutdown = ImmediateShutdown;

    run_session(
        &mut sink,
        &mut observations,
        &mut rpm,
        &mut shutdown,
        SessionConfig::default(),
    )
    .await
    .unwrap();

    let hazard_count = sink
        .frames
        .iter()
        .filter(|frame| {
            ObservedEcuSignal::from_can_frame(frame) == Some(ObservedEcuSignal::HazardButton(true))
        })
        .count();
    assert!(
        hazard_count <= 1,
        "fair scheduler allowed more than one queued hazard ahead of ready shutdown"
    );
    assert_controlled_edges(&sink.frames);
}

#[tokio::test]
async fn observations_emit_independent_strict_can_frames_in_source_order() {
    let mut sink = RecordingSink::default();
    let mut observations = FakeObservations(VecDeque::from([
        Ok(BrokerObservation::HazardButton(true)),
        Ok(BrokerObservation::LeftTurnRequest(true)),
        Ok(BrokerObservation::RightTurnRequest(false)),
        Ok(BrokerObservation::End),
    ]));
    let mut rpm = FakeRpm(VecDeque::new());
    let mut shutdown = PendingShutdown;

    let error = run_session(
        &mut sink,
        &mut observations,
        &mut rpm,
        &mut shutdown,
        SessionConfig::default(),
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("broker stream ended"));
    let observed: Vec<_> = sink
        .frames
        .iter()
        .filter_map(ObservedEcuSignal::from_can_frame)
        .collect();
    assert_eq!(
        observed,
        [
            ObservedEcuSignal::HazardButton(true),
            ObservedEcuSignal::LeftTurnRequest(true),
            ObservedEcuSignal::RightTurnRequest(false),
        ]
    );
    assert_controlled_edges(&sink.frames);
}

#[tokio::test]
async fn broker_end_and_error_write_trailer_then_fail_without_reconnect() {
    for event in [
        Ok(BrokerObservation::End),
        Err(anyhow!("grpc stream failed")),
    ] {
        let mut sink = RecordingSink::default();
        let mut observations = FakeObservations(VecDeque::from([event]));
        let mut rpm = FakeRpm(VecDeque::new());
        let mut shutdown = PendingShutdown;

        let error = run_session(
            &mut sink,
            &mut observations,
            &mut rpm,
            &mut shutdown,
            SessionConfig::default(),
        )
        .await
        .unwrap_err();

        assert_controlled_edges(&sink.frames);
        assert!(error.to_string().contains("broker"));
    }
}

#[tokio::test]
async fn graceful_shutdown_writes_trailer_and_succeeds() {
    let mut sink = RecordingSink::default();
    let mut observations = FakeObservations(VecDeque::new());
    let mut rpm = FakeRpm(VecDeque::new());
    let mut shutdown = ImmediateShutdown;

    run_session(
        &mut sink,
        &mut observations,
        &mut rpm,
        &mut shutdown,
        SessionConfig::default(),
    )
    .await
    .unwrap();

    assert_controlled_edges(&sink.frames);
}

#[tokio::test]
async fn sink_failure_is_reported_without_trailer_retries() {
    let mut sink = RecordingSink {
        fail_at: Some(1),
        ..RecordingSink::default()
    };
    let mut observations =
        FakeObservations(VecDeque::from([Ok(BrokerObservation::HazardButton(true))]));
    let mut rpm = FakeRpm(VecDeque::new());
    let mut shutdown = PendingShutdown;

    let error = run_session(
        &mut sink,
        &mut observations,
        &mut rpm,
        &mut shutdown,
        SessionConfig::default(),
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("injected sink failure"));
    assert_eq!(sink.attempts, 2);
    assert_eq!(sink.frames.len(), 1);
}

#[tokio::test]
async fn initial_connection_failure_emits_nothing() {
    let mut sink = RecordingSink::default();
    let mut rpm = FakeRpm(VecDeque::from([1200]));
    let mut shutdown = PendingShutdown;

    let error = run_connected_session(
        FailedConnector,
        &mut sink,
        &mut rpm,
        &mut shutdown,
        SessionConfig::default(),
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("subscription rejected"));
    assert!(sink.frames.is_empty());
    assert_eq!(sink.attempts, 0);
}

#[tokio::test]
async fn trailer_failure_is_propagated_once_without_power_off_attempt() {
    let mut sink = RecordingSink {
        fail_at: Some(1),
        ..RecordingSink::default()
    };
    let mut observations = FakeObservations(VecDeque::new());
    let mut rpm = FakeRpm(VecDeque::new());
    let mut shutdown = ImmediateShutdown;

    let error = run_session(
        &mut sink,
        &mut observations,
        &mut rpm,
        &mut shutdown,
        SessionConfig::default(),
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("injected sink failure"));
    assert_eq!(sink.attempts, 2);
    assert_eq!(sink.frames.len(), 1);
}
