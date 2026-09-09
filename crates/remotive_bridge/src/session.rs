use crate::source::{HazardConnector, HazardRead, HazardSource, RpmSource, ShutdownSource};
use anyhow::{Context, Result, anyhow};
use common::{ControlSignal, LifecycleCommand, VssSignal};
use emulator::runner::controlled_stop;
use emulator::sink::FrameSink;
use std::num::NonZeroUsize;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SessionConfig {
    pub max_readings: Option<NonZeroUsize>,
}

enum SessionEvent {
    Hazard(Result<HazardRead>),
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
    C: HazardConnector,
    R: RpmSource,
    Stop: ShutdownSource,
{
    let mut hazards = connector.connect().await?;
    run_session(sink, &mut hazards, rpm, shutdown, config).await
}

/// Own the complete bridge write order through one mutable sink.
pub async fn run_session<S, H, R, Stop>(
    sink: &mut S,
    hazards: &mut H,
    rpm: &mut R,
    shutdown: &mut Stop,
    config: SessionConfig,
) -> Result<()>
where
    S: FrameSink,
    H: HazardSource,
    R: RpmSource,
    Stop: ShutdownSource,
{
    sink.write_frame(LifecycleCommand::PowerOn.to_can_frame()?)
        .map_err(|error| anyhow!("write PowerOn: {error}"))?;

    let mut readings = 0usize;
    let mut next_priority = 0u8;
    let terminal_error = loop {
        let event = next_session_event(next_priority, hazards, rpm, shutdown).await;
        next_priority = (next_priority + 1) % 3;

        match event {
            SessionEvent::Hazard(hazard) => match hazard {
                Ok(HazardRead::Value(pressed)) => {
                    sink.write_frame(ControlSignal::HazardButton(pressed).to_can_frame()?)
                        .map_err(|error| anyhow!("write hazard frame: {error}"))?;
                }
                Ok(HazardRead::End) => break Some(anyhow!("broker stream ended")),
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

        // A broker batch can keep `next_hazard` immediately ready. Yield so timers and the
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

async fn next_session_event<H, R, Stop>(
    priority: u8,
    hazards: &mut H,
    rpm: &mut R,
    shutdown: &mut Stop,
) -> SessionEvent
where
    H: HazardSource,
    R: RpmSource,
    Stop: ShutdownSource,
{
    match priority {
        0 => {
            tokio::select! {
                biased;
                value = hazards.next_hazard() => SessionEvent::Hazard(value),
                value = rpm.next_rpm() => SessionEvent::Rpm(value),
                value = shutdown.wait() => SessionEvent::Shutdown(value),
            }
        }
        1 => {
            tokio::select! {
                biased;
                value = rpm.next_rpm() => SessionEvent::Rpm(value),
                value = shutdown.wait() => SessionEvent::Shutdown(value),
                value = hazards.next_hazard() => SessionEvent::Hazard(value),
            }
        }
        _ => {
            tokio::select! {
                biased;
                value = shutdown.wait() => SessionEvent::Shutdown(value),
                value = hazards.next_hazard() => SessionEvent::Hazard(value),
                value = rpm.next_rpm() => SessionEvent::Rpm(value),
            }
        }
    }
}
