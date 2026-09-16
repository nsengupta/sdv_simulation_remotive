use anyhow::Result;
use emulator::sink::SocketCanSink;
use remotive_bridge::cli::parse_args;
use remotive_bridge::session::{SessionConfig, run_connected_session};
use remotive_bridge::source::{CtrlCShutdown, ProfileRpmSource, RemotiveConnector};

#[tokio::main]
async fn main() -> Result<()> {
    let args = parse_args(std::env::args().skip(1))?;

    let mut sink = SocketCanSink::open(&args.can_interface)?;
    let mut rpm = ProfileRpmSource::new(args.tick, args.rpm_clamp);
    let mut shutdown = CtrlCShutdown;

    run_connected_session(
        RemotiveConnector {
            url: args.broker_url,
        },
        &mut sink,
        &mut rpm,
        &mut shutdown,
        SessionConfig {
            max_readings: args.readings,
        },
    )
    .await
}
