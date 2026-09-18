use anyhow::{Context, Result, bail};
use common::RPM_DRIVING_THRESHOLD;
use std::num::NonZeroUsize;
use std::time::Duration;

pub const DEFAULT_BROKER_URL: &str = "http://127.0.0.1:50051";
pub use vehicle_device_bus::DEFAULT_CAN_INTERFACE;
pub const DEFAULT_TICK_MS: u64 = emulator::cli::DEFAULT_TICK_MS;
/// Default RPM ceiling (keeps Twin Idle: Driving needs rpm > this value).
pub const DEFAULT_RPM_CLAMP: u16 = RPM_DRIVING_THRESHOLD;

const USAGE: &str = "\
usage: remotive_bridge [--broker-url <url>] [--can-interface <iface>]
                       [--tick-ms <ms>] [--readings <N>] [--rpm-clamp <rpm>]
";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeArgs {
    pub broker_url: String,
    pub can_interface: String,
    pub tick: Duration,
    pub readings: Option<NonZeroUsize>,
    /// Cap for the bridge RPM profile (`profile.min(rpm_clamp)`).
    pub rpm_clamp: u16,
}

impl Default for BridgeArgs {
    fn default() -> Self {
        Self {
            broker_url: DEFAULT_BROKER_URL.into(),
            can_interface: DEFAULT_CAN_INTERFACE.into(),
            tick: Duration::from_millis(DEFAULT_TICK_MS),
            readings: None,
            rpm_clamp: DEFAULT_RPM_CLAMP,
        }
    }
}

pub fn parse_args<I, S>(args: I) -> Result<BridgeArgs>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let values: Vec<String> = args.into_iter().map(|v| v.as_ref().to_owned()).collect();
    let mut parsed = BridgeArgs::default();
    let mut index = 0;
    while index < values.len() {
        match values[index].as_str() {
            "-h" | "--help" => bail!("{USAGE}"),
            "--broker-url" => {
                index += 1;
                parsed.broker_url = non_empty(values.get(index), "--broker-url")?.to_owned();
            }
            "--can-interface" => {
                index += 1;
                parsed.can_interface = non_empty(values.get(index), "--can-interface")?.to_owned();
            }
            "--tick-ms" => {
                index += 1;
                let raw = values.get(index).context("missing value for --tick-ms")?;
                let millis = raw
                    .parse::<u64>()
                    .with_context(|| format!("invalid --tick-ms value {raw:?}"))?;
                if millis == 0 {
                    bail!("--tick-ms must be greater than zero");
                }
                parsed.tick = Duration::from_millis(millis);
            }
            "--readings" => {
                index += 1;
                let raw = values.get(index).context("missing value for --readings")?;
                let count = raw
                    .parse::<usize>()
                    .with_context(|| format!("invalid --readings value {raw:?}"))?;
                parsed.readings =
                    Some(NonZeroUsize::new(count).context("--readings must be greater than zero")?);
            }
            "--rpm-clamp" => {
                index += 1;
                let raw = values.get(index).context("missing value for --rpm-clamp")?;
                parsed.rpm_clamp = raw
                    .parse::<u16>()
                    .with_context(|| format!("invalid --rpm-clamp value {raw:?}"))?;
            }
            other => bail!("unknown argument {other:?}\n{USAGE}"),
        }
        index += 1;
    }
    Ok(parsed)
}

fn non_empty<'a>(value: Option<&'a String>, flag: &str) -> Result<&'a str> {
    let value = value.with_context(|| format!("missing value for {flag}"))?;
    if value.is_empty() {
        bail!("{flag} requires a non-empty value");
    }
    Ok(value)
}
