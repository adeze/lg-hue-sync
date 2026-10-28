use mdns_sd::{DaemonEvent, IfKind, ServiceDaemon, ServiceInfo};
use std::time::Duration;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

fn service_info(port: u16) -> mdns_sd::Result<ServiceInfo> {
    ServiceInfo::new(
        "_http._tcp.local.",
        "LG Hue Sync",
        "lg-hue-sync.local.",
        "",
        port,
        &[
            ("path", "/"),
            ("openapi", "/openapi.json"),
            ("app", "lg-hue-sync"),
            ("version", env!("CARGO_PKG_VERSION")),
        ][..],
    )
    .map(ServiceInfo::enable_addr_auto)
}

pub(super) async fn manage(
    port: u16,
    mut enabled: watch::Receiver<bool>,
    shutdown: CancellationToken,
) {
    loop {
        let advertise_now = *enabled.borrow_and_update();
        if advertise_now {
            advertise(port, &mut enabled, shutdown.clone()).await;
        }
        tokio::select! {
            _ = shutdown.cancelled() => break,
            changed = enabled.changed() => if changed.is_err() { break; },
        }
    }
}

async fn advertise(port: u16, enabled: &mut watch::Receiver<bool>, shutdown: CancellationToken) {
    let daemon = match ServiceDaemon::new() {
        Ok(daemon) => daemon,
        Err(error) => {
            warn!("Bonjour advertising unavailable: {error}");
            return;
        }
    };
    let result: anyhow::Result<()> = async {
        // The HTTP listener binds IPv4 only; do not advertise unreachable IPv6 addresses.
        daemon.disable_interface(IfKind::IPv6)?;
        let events = daemon.monitor()?;
        daemon.register(service_info(port)?)?;
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => break,
                changed = enabled.changed() => {
                    if changed.is_err() || !*enabled.borrow_and_update() { break; }
                },
                event = events.recv_async() => match event? {
                    DaemonEvent::Announce(_, _) => info!("Bonjour HTTP service announced"),
                    DaemonEvent::Error(error) => warn!("Bonjour advertising error: {error}"),
                    DaemonEvent::NameChange(_) => info!("Bonjour resolved a name conflict"),
                    _ => {}
                }
            }
        }
        Ok(())
    }
    .await;
    if let Err(error) = result {
        warn!("Bonjour advertising stopped: {error}");
    }
    if let Ok(unregistered) = daemon.unregister("LG Hue Sync._http._tcp.local.") {
        let _ = tokio::time::timeout(Duration::from_secs(1), unregistered.recv_async()).await;
    }
    if let Ok(stopped) = daemon.shutdown() {
        let _ = tokio::time::timeout(Duration::from_secs(1), stopped.recv_async()).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advertisement_links_dashboard_and_openapi_on_the_bound_port() {
        let service = service_info(12345).unwrap();
        assert_eq!(service.get_fullname(), "LG Hue Sync._http._tcp.local.");
        assert_eq!(service.get_hostname(), "lg-hue-sync.local.");
        assert_eq!(service.get_port(), 12345);
        assert!(service.is_addr_auto());
        for (key, value) in [
            ("path", "/"),
            ("openapi", "/openapi.json"),
            ("app", "lg-hue-sync"),
            ("version", env!("CARGO_PKG_VERSION")),
        ] {
            assert_eq!(service.get_property_val_str(key), Some(value));
        }
        assert_eq!(service.get_properties().len(), 4);
    }
}
