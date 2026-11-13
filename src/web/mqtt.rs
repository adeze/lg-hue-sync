//! Optional publisher for bounded QSync status snapshots.
use rumqttc::{
    AsyncClient, Event, LastWill, MqttOptions, Outgoing, Packet, QoS, TlsConfiguration, Transport,
};
use serde::Deserialize;
use std::time::Duration;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Deserialize, serde::Serialize)]
pub(crate) struct MqttConfig {
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    pub client_id: String,
    #[serde(default)]
    pub tls: bool,
    pub username: Option<String>,
    pub password: Option<String>,
}

impl std::fmt::Debug for MqttConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("MqttConfig").finish_non_exhaustive()
    }
}

fn default_port() -> u16 {
    1883
}

pub(super) async fn publish(
    config: MqttConfig,
    mut changes: watch::Receiver<()>,
    snapshot: impl Fn() -> serde_json::Value,
    shutdown: CancellationToken,
) {
    if config.host.trim().is_empty()
        || config.port == 0
        || config.client_id.is_empty()
        || config.client_id.len() > 64
        || !config
            .client_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
        || (!config.tls && (config.username.is_some() || config.password.is_some()))
        || config.username.is_some() != config.password.is_some()
    {
        tracing::warn!("Invalid MQTT configuration. Credentials require TLS.");
        return;
    }
    let status_topic = format!("qsync/{}/status", config.client_id);
    let availability = format!("qsync/{}/availability", config.client_id);
    let mut options = MqttOptions::new(config.client_id, config.host, config.port);
    options
        .set_keep_alive(Duration::from_secs(15))
        .set_last_will(LastWill::new(
            &availability,
            "offline",
            QoS::AtLeastOnce,
            true,
        ));
    if config.tls {
        options.set_transport(Transport::tls_with_config(TlsConfiguration::Native));
    }
    if let (Some(username), Some(password)) = (config.username, config.password) {
        options.set_credentials(username, password);
    }
    let (client, mut connection) = AsyncClient::new(options, 4);
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last = None;
    let mut dirty = true;
    let mut connected = false;
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => break,
            result = connection.poll() => match result {
                Ok(Event::Incoming(Packet::ConnAck(_))) => {
                    connected = true;
                    dirty = true;
                    last = None;
                    let _ = client.try_publish(&availability, QoS::AtLeastOnce, true, "online");
                }
                Ok(_) => {}
                Err(_) => {
                    if connected {
                        tracing::warn!("QSync MQTT broker connection lost");
                    }
                    connected = false;
                    tokio::select! {
                        _ = shutdown.cancelled() => return,
                        _ = tokio::time::sleep(Duration::from_secs(5)) => {}
                    }
                }
            },
            result = changes.changed() => {
                if result.is_err() {
                    break;
                }
                dirty = true;
            },
            _ = tick.tick(), if connected && dirty => {
                let current = snapshot();
                if last.as_ref() == Some(&current) {
                    dirty = false;
                } else if client.try_publish(&status_topic, QoS::AtLeastOnce, true, current.to_string()).is_ok() {
                    last = Some(current);
                    dirty = false;
                }
            }
        }
    }
    if connected
        && client
            .try_publish(&availability, QoS::AtLeastOnce, true, "offline")
            .is_ok()
        && client.try_disconnect().is_ok()
    {
        let _ = tokio::time::timeout(Duration::from_secs(2), async {
            while let Ok(event) = connection.poll().await {
                if matches!(event, Event::Outgoing(Outgoing::Disconnect)) {
                    break;
                }
            }
        })
        .await;
    }
}
