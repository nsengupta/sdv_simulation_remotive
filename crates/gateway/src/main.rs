//! Gateway — sole Digital Twin owner: CAN ingress, actuation, observation tee, optional live UDS.

use std::io::IsTerminal;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use common::DiagnosticKind;
use common::facade::{DiagnosticRecord, PublishedTransitionRecord};
use observation::{
    AnyLiveSink, LiveMessage, ObservationTee, RunId, RunMetadata, UdsLiveSink, UnixTimestampV1,
    ZenohLiveSink,
};
use tokio::sync::mpsc;

use gateway::cli::{self, GatewayArgs, GatewayLiveMode};
use gateway::gateway_runtime::{self, TwinRuntimeBuilder};
use gateway::transition_log;

const VIRTUAL_CAR_IDENTITY: &str = "My-Opel-Corsa-1.4-GSi";
const BOOT_DIAGNOSTIC_WAIT: Duration = Duration::from_secs(2);

#[tokio::main]
async fn main() -> Result<()> {
    let args = cli::parse_args(std::env::args_os().skip(1))?;
    if args.print_transitions_only {
        return run_ledger_only_mode(args.trace_actuation_ingress).await;
    }
    run_with_capture(args).await
}

async fn run_ledger_only_mode(trace_actuation_ingress: bool) -> Result<()> {
    let mut builder = TwinRuntimeBuilder::new()
        .with_car_identity(VIRTUAL_CAR_IDENTITY)
        .with_can_interface(gateway_runtime::DEFAULT_CAN_INTERFACE)
        .with_ingress_console_log(true);

    if trace_actuation_ingress {
        builder = builder.with_actuation_ingress_trace();
    }

    let (trans_tx, trans_rx) = mpsc::channel::<PublishedTransitionRecord>(256);
    let color = std::io::stdout().is_terminal();
    let _trans_log = transition_log::spawn_transition_log_task(trans_rx, color);
    builder = builder.with_transition_channel(trans_tx);
    eprintln!("[gateway] ledger-only mode (--print-transitions-only); colours={color}");
    builder.run().await
}

async fn run_with_capture(args: GatewayArgs) -> Result<()> {
    let mut builder = TwinRuntimeBuilder::new()
        .with_car_identity(VIRTUAL_CAR_IDENTITY)
        .with_can_interface(gateway_runtime::DEFAULT_CAN_INTERFACE)
        .with_auto_power_on(false)
        .with_ingress_console_log(true);

    if args.trace_actuation_ingress {
        builder = builder.with_actuation_ingress_trace();
    }

    let (diag_tx, mut diag_rx) = mpsc::unbounded_channel::<DiagnosticRecord>();
    let (trans_tx, mut trans_rx) = mpsc::channel::<PublishedTransitionRecord>(256);
    builder = builder
        .with_diagnostic_channel(diag_tx)
        .with_transition_channel(trans_tx);

    let live_sink: Option<AnyLiveSink> = match &args.live {
        GatewayLiveMode::Uds(uds_path) => {
            eprintln!(
                "[gateway] waiting for Dashboard on {} (timeout {:?})",
                uds_path.display(),
                args.connect_timeout
            );
            Some(AnyLiveSink::Uds(
                UdsLiveSink::bind_and_accept(
                    uds_path.clone(),
                    args.connect_timeout,
                    LiveMessage::hello(VIRTUAL_CAR_IDENTITY),
                )
                .await
                .context("UDS accept / hello")?,
            ))
        }
        GatewayLiveMode::Zenoh { keyexpr } => {
            eprintln!(
                "[gateway] waiting for Zenoh subscriber on {keyexpr} (timeout {:?})",
                args.connect_timeout
            );
            Some(AnyLiveSink::Zenoh(
                ZenohLiveSink::open_and_wait_subscriber(
                    keyexpr.clone(),
                    args.connect_timeout,
                    LiveMessage::hello(VIRTUAL_CAR_IDENTITY),
                )
                .await
                .context("Zenoh subscriber wait / hello")?,
            ))
        }
        GatewayLiveMode::NoLive => None,
    };

    let (controller, _opts) = builder.install_controller().await?;
    let boot = require_boot_diagnostic(&mut diag_rx, BOOT_DIAGNOSTIC_WAIT).await?;
    let metadata = RunMetadata::now(
        RunId::new_v4(),
        UnixTimestampV1::from_live(boot.session_started_at),
        VIRTUAL_CAR_IDENTITY,
        None,
    )?;
    let writer = observation::RunWriter::create(&args.observation_dir, metadata)?;
    eprintln!("[gateway] Observation run: {}", writer.run_dir().display());

    let mut tee = ObservationTee::new(writer, live_sink);
    tee.record_diagnostic(&boot)
        .context("persist boot diagnostic")?;

    let mut runtime_handle = builder.spawn_runtime(controller)?;

    loop {
        tokio::select! {
            biased;
            diagnostic = diag_rx.recv() => {
                match diagnostic {
                    Some(record) => {
                        if std::io::stdout().is_terminal() {
                            println!("{record}");
                        }
                        tee.record_diagnostic(&record)
                            .context("tee diagnostic")?;
                    }
                    None => break,
                }
            }
            transition = trans_rx.recv() => {
                match transition {
                    Some(record) => {
                        tee.record_ledger(&record).context("tee ledger")?;
                    }
                    None => break,
                }
            }
            join = &mut runtime_handle => {
                match join {
                    Ok(Ok(())) => break,
                    Ok(Err(err)) => {
                        let _ = tee.finish();
                        return Err(err);
                    }
                    Err(err) => {
                        let _ = tee.finish();
                        return Err(err.into());
                    }
                }
            }
        }
    }

    tee.finish().context("finish observation capture")?;
    Ok(())
}

async fn require_boot_diagnostic(
    rx: &mut mpsc::UnboundedReceiver<DiagnosticRecord>,
    wait: Duration,
) -> Result<DiagnosticRecord> {
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            bail!("timed out waiting for Twin boot diagnostic");
        }
        match tokio::time::timeout(remaining, rx.recv()).await {
            Ok(Some(record)) if matches!(record.kind, DiagnosticKind::Boot) => {
                return Ok(record);
            }
            Ok(Some(_)) => continue,
            Ok(None) => bail!("diagnostic channel closed before boot"),
            Err(_) => bail!("timed out waiting for Twin boot diagnostic"),
        }
    }
}
