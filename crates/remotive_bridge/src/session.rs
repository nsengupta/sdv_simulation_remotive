use crate::source::{
    BrokerObservation, ObservationConnector, ObservationSource, RpmSource, ShutdownSource,
};
use anyhow::{Context, Result, anyhow};
use common::{LifecycleCommand, ObservedEcuSignal, VssSignal};
use emulator::runner::controlled_stop;
use emulator::sink::FrameSink;
use std::num::NonZeroUsize;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SessionConfig {
    pub max_readings: Option<NonZeroUsize>,
}

enum SessionEvent {
    Observation(Result<BrokerObservation>),
    Rpm(Result<u16>),
    Shutdown(Result<()>),
}

pub async fn run_connected_session<S, C, R, Stop>(
    connector: C,
    sink: &mut S,
    rpm: &mut R,
    shutdown: &mut Stop,
    config: SessionConfig,
) -> Result<()>
where
    S: FrameSink,
    C: ObservationConnector,
    R: RpmSource,
    Stop: ShutdownSource,
{
    let mut observations = connector.connect().await?;
    run_session(sink, &mut observations, rpm, shutdown, config).await
}

/// Own the complete bridge write order through one mutable sink.
pub async fn run_session<S, O, R, Stop>(
    sink: &mut S,
    observations: &mut O,
    rpm: &mut R,
    shutdown: &mut Stop,
    config: SessionConfig,
) -> Result<()>
where
    S: FrameSink,
    O: ObservationSource,
    R: RpmSource,
    Stop: ShutdownSource,
{
    sink.write_frame(LifecycleCommand::PowerOn.to_can_frame()?)
        .map_err(|error| anyhow!("write PowerOn: {error}"))?;

    let mut readings = 0usize;
    let mut next_priority = 0u8;
    let terminal_error = loop {
        let event = next_session_event(next_priority, observations, rpm, shutdown).await;
        next_priority = (next_priority + 1) % 3;

        match event {
            SessionEvent::Observation(observation) => match observation {
                Ok(BrokerObservation::HazardButton(value)) => {
                    sink.write_frame(ObservedEcuSignal::HazardButton(value).to_can_frame()?)
                        .map_err(|error| anyhow!("write hazard observation frame: {error}"))?;
                }
                Ok(BrokerObservation::LeftTurnRequest(value)) => {
                    sink.write_frame(ObservedEcuSignal::LeftTurnRequest(value).to_can_frame()?)
                        .map_err(|error| anyhow!("write left-turn observation frame: {error}"))?;
                }
                Ok(BrokerObservation::RightTurnRequest(value)) => {
                    sink.write_frame(ObservedEcuSignal::RightTurnRequest(value).to_can_frame()?)
                        .map_err(|error| anyhow!("write right-turn observation frame: {error}"))?;
                }
                Ok(BrokerObservation::End) => break Some(anyhow!("broker stream ended")),
                Err(error) => break Some(error.context("broker stream failed")),
            },
            SessionEvent::Rpm(next_rpm) => {
                let value = next_rpm.context("read RPM profile")?;
                sink.write_frame(VssSignal::EngineRpm(value).to_can_frame()?)
                    .map_err(|error| anyhow!("write RPM frame: {error}"))?;
                readings += 1;
                if config
                    .max_readings
                    .is_some_and(|limit| readings >= limit.get())
                {
                    break None;
                }
            }
            SessionEvent::Shutdown(stopped) => {
                stopped.context("wait for shutdown")?;
                break None;
            }
        }

        // A broker batch can keep `next_observation` immediately ready. Yield so timers and the
        // Ctrl+C driver can become ready before the next bounded-priority selection.
        tokio::task::yield_now().await;
    };

    controlled_stop(sink)
        .map_err(|error| anyhow!("write controlled RPM0/PowerOff trailer: {error}"))?;
    match terminal_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

async fn next_session_event<O, R, Stop>(
    priority: u8,
    observations: &mut O,
    rpm: &mut R,
    shutdown: &mut Stop,
) -> SessionEvent
where
    O: ObservationSource,
    R: RpmSource,
    Stop: ShutdownSource,
{
    match priority {
        0 => {
            tokio::select! {
                biased;
                value = observations.next_observation() => SessionEvent::Observation(value),
                value = rpm.next_rpm() => SessionEvent::Rpm(value),
                value = shutdown.wait() => SessionEvent::Shutdown(value),
            }
        }
        1 => {
            tokio::select! {
                biased;
                value = rpm.next_rpm() => SessionEvent::Rpm(value),
                value = shutdown.wait() => SessionEvent::Shutdown(value),
                value = observations.next_observation() => SessionEvent::Observation(value),
            }
        }
        _ => {
            tokio::select! {
                biased;
                value = shutdown.wait() => SessionEvent::Shutdown(value),
                value = observations.next_observation() => SessionEvent::Observation(value),
                value = rpm.next_rpm() => SessionEvent::Rpm(value),
            }
        }
    }
}
