use crate::decoder::{decode_boolean, decode_ok_fail};
use anyhow::{Context, Result};
use async_trait::async_trait;
use emulator::models::{PhysicalWorldModelConfig, RpmModel};
use remotivelabs_broker::{
    Connection,
    generated::base::{
        ClientId, NameSpace, Signal, SignalId, SignalIds, Signals, SubscriberConfig,
        signal::Payload,
    },
};
use std::collections::VecDeque;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tonic::Streaming;

pub const CLIENT_ID: &str = "sdv-remotive-bridge";
pub const HAZARD_NAMESPACE: &str = "SCCM-DriverCan0";
pub const HAZARD_NAME: &str = "HazardLightButton.HazardLightButton";
pub const TURN_NAMESPACE: &str = "BCM-BodyCan0";
pub const LEFT_TURN_NAME: &str = "TurnLightControl.LeftTurnLightRequest";
pub const RIGHT_TURN_NAME: &str = "TurnLightControl.RightTurnLightRequest";
pub const FLCM_NAMESPACE: &str = "FLCM-BodyCan0";
pub const LEFT_LOW_BEAM_STATUS_NAME: &str = "LowBeamLightStatus.LeftLowBeamLightStatus";
pub const RIGHT_LOW_BEAM_STATUS_NAME: &str = "LowBeamLightStatus.RightLowBeamLightStatus";

const SUBSCRIBED_SIGNALS: [(&str, &str); 5] = [
    (HAZARD_NAMESPACE, HAZARD_NAME),
    (TURN_NAMESPACE, LEFT_TURN_NAME),
    (TURN_NAMESPACE, RIGHT_TURN_NAME),
    (FLCM_NAMESPACE, LEFT_LOW_BEAM_STATUS_NAME),
    (FLCM_NAMESPACE, RIGHT_LOW_BEAM_STATUS_NAME),
];

pub fn subscription_ready_status() -> String {
    let signals = SUBSCRIBED_SIGNALS
        .iter()
        .map(|(namespace, name)| format!("{namespace}:{name}"))
        .collect::<Vec<_>>()
        .join(",\n");
    format!("[remotive_bridge] connected; subscribed signals=\n{signals}")
}

pub fn subscription_config() -> SubscriberConfig {
    SubscriberConfig {
        client_id: Some(ClientId {
            id: CLIENT_ID.to_owned(),
        }),
        signals: Some(SignalIds {
            signal_id: SUBSCRIBED_SIGNALS
                .into_iter()
                .map(|(namespace, name)| SignalId {
                    name: name.to_owned(),
                    namespace: Some(NameSpace {
                        name: namespace.to_owned(),
                    }),
                })
                .collect(),
        }),
        on_change: false,
        initial_empty: false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrokerObservation {
    HazardButton(bool),
    LeftTurnRequest(bool),
    RightTurnRequest(bool),
    LeftLowBeamStatus(bool),
    RightLowBeamStatus(bool),
    End,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ObservationRejectCounters {
    wrong_identity: u64,
    invalid_payload: u64,
}

impl ObservationRejectCounters {
    pub fn wrong_identity(&self) -> u64 {
        self.wrong_identity
    }

    pub fn invalid_payload(&self) -> u64 {
        self.invalid_payload
    }

    fn reject_identity(&mut self) {
        self.wrong_identity = self.wrong_identity.saturating_add(1);
        report_rejection_count("wrong signal identity", self.wrong_identity);
    }

    fn reject_payload(&mut self) {
        self.invalid_payload = self.invalid_payload.saturating_add(1);
        report_rejection_count("invalid observation payload", self.invalid_payload);
    }
}

fn report_rejection_count(reason: &str, count: u64) {
    if count.is_power_of_two() {
        eprintln!("[remotive_bridge] rejected {reason}; count={count}");
    }
}

fn observation_kind(
    signal: &Signal,
) -> Option<(
    fn(bool) -> BrokerObservation,
    fn(Option<&Payload>) -> Option<bool>,
)> {
    let id = signal.id.as_ref()?;
    let namespace = id.namespace.as_ref()?.name.as_str();
    match (namespace, id.name.as_str()) {
        (HAZARD_NAMESPACE, HAZARD_NAME) => Some((BrokerObservation::HazardButton, decode_boolean)),
        (TURN_NAMESPACE, LEFT_TURN_NAME) => {
            Some((BrokerObservation::LeftTurnRequest, decode_boolean))
        }
        (TURN_NAMESPACE, RIGHT_TURN_NAME) => {
            Some((BrokerObservation::RightTurnRequest, decode_boolean))
        }
        (FLCM_NAMESPACE, LEFT_LOW_BEAM_STATUS_NAME) => {
            Some((BrokerObservation::LeftLowBeamStatus, decode_ok_fail))
        }
        (FLCM_NAMESPACE, RIGHT_LOW_BEAM_STATUS_NAME) => {
            Some((BrokerObservation::RightLowBeamStatus, decode_ok_fail))
        }
        _ => None,
    }
}

pub fn decode_observation_signals(
    signals: &Signals,
    rejected: &mut ObservationRejectCounters,
) -> Vec<BrokerObservation> {
    signals
        .signal
        .iter()
        .filter_map(|signal| {
            let Some((build, decode)) = observation_kind(signal) else {
                rejected.reject_identity();
                return None;
            };
            match decode(signal.payload.as_ref()) {
                Some(value) => Some(build(value)),
                None => {
                    rejected.reject_payload();
                    None
                }
            }
        })
        .collect()
}

#[async_trait]
pub trait ObservationSource {
    async fn next_observation(&mut self) -> Result<BrokerObservation>;
}

#[async_trait]
pub trait ObservationConnector {
    type Source: ObservationSource;

    async fn connect(self) -> Result<Self::Source>;
}

#[async_trait]
pub trait RpmSource {
    async fn next_rpm(&mut self) -> Result<u16>;
}

#[async_trait]
pub trait ShutdownSource {
    async fn wait(&mut self) -> Result<()>;
}

pub struct RemotiveObservationSource {
    stream: Streaming<Signals>,
    pending: VecDeque<BrokerObservation>,
    rejected: ObservationRejectCounters,
}

impl RemotiveObservationSource {
    /// Connect and establish all subscriptions before any lifecycle frame is emitted.
    pub async fn connect(url: String) -> Result<Self> {
        let mut connection = Connection::new(url, None)
            .await
            .map_err(|error| anyhow::anyhow!("connect to Remotive broker: {error}"))?;
        let stream = connection
            .network_stub
            .subscribe_to_signals(subscription_config())
            .await
            .context("establish Remotive observation subscription")?
            .into_inner();
        eprintln!("{}", subscription_ready_status());
        Ok(Self {
            stream,
            pending: VecDeque::new(),
            rejected: ObservationRejectCounters::default(),
        })
    }
}

#[async_trait]
impl ObservationSource for RemotiveObservationSource {
    async fn next_observation(&mut self) -> Result<BrokerObservation> {
        loop {
            if let Some(observation) = self.pending.pop_front() {
                return Ok(observation);
            }
            let Some(signals) = self.stream.message().await.context("broker stream error")? else {
                return Ok(BrokerObservation::End);
            };
            self.pending
                .extend(decode_observation_signals(&signals, &mut self.rejected));
        }
    }
}

pub struct RemotiveConnector {
    pub url: String,
}

#[async_trait]
impl ObservationConnector for RemotiveConnector {
    type Source = RemotiveObservationSource;

    async fn connect(self) -> Result<Self::Source> {
        RemotiveObservationSource::connect(self.url).await
    }
}

pub struct ProfileRpmSource {
    model: RpmModel,
    current_rpm: u16,
    rpm_clamp: u16,
    interval: tokio::time::Interval,
}

impl ProfileRpmSource {
    pub fn new(tick: Duration, rpm_clamp: u16) -> Self {
        let config = PhysicalWorldModelConfig::daytime_tunnel_profile().rpm;
        let current_rpm = config.idle_rpm.min(rpm_clamp);
        Self {
            model: RpmModel::new(config),
            current_rpm,
            rpm_clamp,
            interval: tokio::time::interval(tick),
        }
    }
}

#[async_trait]
impl RpmSource for ProfileRpmSource {
    async fn next_rpm(&mut self) -> Result<u16> {
        self.interval.tick().await;
        let epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("system clock is before Unix epoch")?
            .as_secs();
        self.current_rpm = self
            .model
            .next_rpm(self.current_rpm, epoch)
            .min(self.rpm_clamp);
        Ok(self.current_rpm)
    }
}

pub struct CtrlCShutdown;

#[async_trait]
impl ShutdownSource for CtrlCShutdown {
    async fn wait(&mut self) -> Result<()> {
        tokio::signal::ctrl_c()
            .await
            .context("install Ctrl+C handler")
    }
}
