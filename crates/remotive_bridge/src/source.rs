use crate::decoder::decode_hazard;
use anyhow::{Context, Result};
use async_trait::async_trait;
use emulator::models::{PhysicalWorldModelConfig, RpmModel};
use remotivelabs_broker::{
    Connection,
    generated::base::{ClientId, NameSpace, SignalId, SignalIds, Signals, SubscriberConfig},
};
use std::collections::VecDeque;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tonic::Streaming;

pub const CLIENT_ID: &str = "sdv-remotive-bridge";
pub const HAZARD_NAMESPACE: &str = "SCCM-DriverCan0";
pub const HAZARD_NAME: &str = "HazardLightButton.HazardLightButton";

pub fn subscription_ready_status() -> String {
    format!("[remotive_bridge] connected; subscribed signal={HAZARD_NAMESPACE}:{HAZARD_NAME}")
}

pub fn subscription_config() -> SubscriberConfig {
    SubscriberConfig {
        client_id: Some(ClientId {
            id: CLIENT_ID.to_owned(),
        }),
        signals: Some(SignalIds {
            signal_id: vec![SignalId {
                name: HAZARD_NAME.to_owned(),
                namespace: Some(NameSpace {
                    name: HAZARD_NAMESPACE.to_owned(),
                }),
            }],
        }),
        on_change: false,
        initial_empty: false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HazardRead {
    Value(bool),
    End,
}

#[async_trait]
pub trait HazardSource {
    async fn next_hazard(&mut self) -> Result<HazardRead>;
}

#[async_trait]
pub trait HazardConnector {
    type Source: HazardSource;

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

pub struct RemotiveHazardSource {
    stream: Streaming<Signals>,
    pending: VecDeque<bool>,
}

impl RemotiveHazardSource {
    /// Connect and establish the subscription before any lifecycle frame is emitted.
    pub async fn connect(url: String) -> Result<Self> {
        let mut connection = Connection::new(url, None)
            .await
            .map_err(|error| anyhow::anyhow!("connect to Remotive broker: {error}"))?;
        let stream = connection
            .network_stub
            .subscribe_to_signals(subscription_config())
            .await
            .context("establish Remotive hazard subscription")?
            .into_inner();
        eprintln!("{}", subscription_ready_status());
        Ok(Self {
            stream,
            pending: VecDeque::new(),
        })
    }
}

pub struct RemotiveConnector {
    pub url: String,
}

#[async_trait]
impl HazardConnector for RemotiveConnector {
    type Source = RemotiveHazardSource;

    async fn connect(self) -> Result<Self::Source> {
        RemotiveHazardSource::connect(self.url).await
    }
}

#[async_trait]
impl HazardSource for RemotiveHazardSource {
    async fn next_hazard(&mut self) -> Result<HazardRead> {
        loop {
            if let Some(value) = self.pending.pop_front() {
                return Ok(HazardRead::Value(value));
            }
            let Some(signals) = self.stream.message().await.context("broker stream error")? else {
                return Ok(HazardRead::End);
            };
            self.pending.extend(
                signals
                    .signal
                    .iter()
                    .filter_map(|signal| decode_hazard(signal.payload.as_ref())),
            );
        }
    }
}

pub struct ProfileRpmSource {
    model: RpmModel,
    current_rpm: u16,
    interval: tokio::time::Interval,
}

impl ProfileRpmSource {
    pub fn new(tick: Duration) -> Self {
        let config = PhysicalWorldModelConfig::daytime_tunnel_profile().rpm;
        let current_rpm = config.idle_rpm;
        Self {
            model: RpmModel::new(config),
            current_rpm,
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
        self.current_rpm = self.model.next_rpm(self.current_rpm, epoch);
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
