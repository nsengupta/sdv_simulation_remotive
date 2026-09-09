use anyhow::{Result, anyhow};
use async_trait::async_trait;
use common::{ControlSignal, LifecycleCommand, VssSignal};
use emulator::sink::FrameSink;
use remotive_bridge::session::{SessionConfig, run_connected_session, run_session};
use remotive_bridge::source::{
    HazardConnector, HazardRead, HazardSource, RpmSource, ShutdownSource,
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

struct FakeHazards(VecDeque<Result<HazardRead>>);

#[async_trait]
impl HazardSource for FakeHazards {
    async fn next_hazard(&mut self) -> Result<HazardRead> {
        match self.0.pop_front() {
            Some(item) => item,
            None => pending().await,
        }
    }
}

struct RepeatingHazards {
    remaining: usize,
}

#[async_trait]
impl HazardSource for RepeatingHazards {
    async fn next_hazard(&mut self) -> Result<HazardRead> {
        if self.remaining == 0 {
            pending().await
        } else {
            self.remaining -= 1;
            Ok(HazardRead::Value(true))
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
impl HazardConnector for FailedConnector {
    type Source = FakeHazards;

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
    let mut hazards = FakeHazards(VecDeque::from([Ok(HazardRead::Value(true))]));
    let mut rpm = FakeRpm(VecDeque::from([1100, 1200]));
    let mut shutdown = PendingShutdown;

    run_session(
        &mut sink,
        &mut hazards,
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
        ControlSignal::from_can_frame(frame) == Some(ControlSignal::HazardButton(true))
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
    let mut hazards = RepeatingHazards { remaining: 64 };
    let mut rpm = FakeRpm(VecDeque::from([1200]));
    let mut shutdown = PendingShutdown;

    run_session(
        &mut sink,
        &mut hazards,
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
            ControlSignal::from_can_frame(frame) == Some(ControlSignal::HazardButton(true))
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
    let mut hazards = RepeatingHazards { remaining: 64 };
    let mut rpm = FakeRpm(VecDeque::new());
    let mut shutdown = ImmediateShutdown;

    run_session(
        &mut sink,
        &mut hazards,
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
            ControlSignal::from_can_frame(frame) == Some(ControlSignal::HazardButton(true))
        })
        .count();
    assert!(
        hazard_count <= 1,
        "fair scheduler allowed more than one queued hazard ahead of ready shutdown"
    );
    assert_controlled_edges(&sink.frames);
}

#[tokio::test]
async fn duplicate_hazards_each_emit_a_strict_can_frame() {
    let mut sink = RecordingSink::default();
    let mut hazards = FakeHazards(VecDeque::from([
        Ok(HazardRead::Value(true)),
        Ok(HazardRead::Value(true)),
        Ok(HazardRead::End),
    ]));
    let mut rpm = FakeRpm(VecDeque::new());
    let mut shutdown = PendingShutdown;

    let error = run_session(
        &mut sink,
        &mut hazards,
        &mut rpm,
        &mut shutdown,
        SessionConfig::default(),
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("broker stream ended"));
    assert_eq!(
        sink.frames
            .iter()
            .filter(|frame| ControlSignal::from_can_frame(frame)
                == Some(ControlSignal::HazardButton(true)))
            .count(),
        2
    );
    assert_controlled_edges(&sink.frames);
}

#[tokio::test]
async fn broker_end_and_error_write_trailer_then_fail_without_reconnect() {
    for event in [Ok(HazardRead::End), Err(anyhow!("grpc stream failed"))] {
        let mut sink = RecordingSink::default();
        let mut hazards = FakeHazards(VecDeque::from([event]));
        let mut rpm = FakeRpm(VecDeque::new());
        let mut shutdown = PendingShutdown;

        let error = run_session(
            &mut sink,
            &mut hazards,
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
    let mut hazards = FakeHazards(VecDeque::new());
    let mut rpm = FakeRpm(VecDeque::new());
    let mut shutdown = ImmediateShutdown;

    run_session(
        &mut sink,
        &mut hazards,
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
    let mut hazards = FakeHazards(VecDeque::from([Ok(HazardRead::Value(true))]));
    let mut rpm = FakeRpm(VecDeque::new());
    let mut shutdown = PendingShutdown;

    let error = run_session(
        &mut sink,
        &mut hazards,
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
    let mut hazards = FakeHazards(VecDeque::new());
    let mut rpm = FakeRpm(VecDeque::new());
    let mut shutdown = ImmediateShutdown;

    let error = run_session(
        &mut sink,
        &mut hazards,
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
