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
    let terminal_error = loop {
        tokio::select! {
            biased;
            hazard = hazards.next_hazard() => {
                match hazard {
                    Ok(HazardRead::Value(pressed)) => {
                        sink.write_frame(ControlSignal::HazardButton(pressed).to_can_frame()?)
                            .map_err(|error| anyhow!("write hazard frame: {error}"))?;
                    }
                    Ok(HazardRead::End) => break Some(anyhow!("broker stream ended")),
                    Err(error) => break Some(error.context("broker stream failed")),
                }
            }
            next_rpm = rpm.next_rpm() => {
                let value = next_rpm.context("read RPM profile")?;
                sink.write_frame(VssSignal::EngineRpm(value).to_can_frame()?)
                    .map_err(|error| anyhow!("write RPM frame: {error}"))?;
                readings += 1;
                if config.max_readings.is_some_and(|limit| readings >= limit.get()) {
                    break None;
                }
            }
            stopped = shutdown.wait() => {
                stopped.context("wait for shutdown")?;
                break None;
            }
        }
    };

    controlled_stop(sink)
        .map_err(|error| anyhow!("write controlled RPM0/PowerOff trailer: {error}"))?;
    match terminal_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}
