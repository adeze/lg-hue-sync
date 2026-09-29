use crate::config::{Config, NanoleafAlignment, NanoleafConfig};
use crate::{hue, nanoleaf};
use anyhow::Result;
use std::path::PathBuf;
use tracing::info;

pub(crate) async fn run_pair(bridge_opt: Option<String>, output: PathBuf) -> Result<()> {
    let bridge_ip = match bridge_opt {
        Some(ip) => ip,
        None => match hue::discover_bridge() {
            Ok(ip) => ip,
            Err(e) => {
                println!(
                    "Auto-discovery: {}. Please enter Hue Bridge IP manually:",
                    e
                );
                use std::io::{stdin, stdout, Write};
                print!("Bridge IP: ");
                stdout().flush().ok();
                let mut line = String::new();
                stdin().read_line(&mut line)?;
                line.trim().to_string()
            }
        },
    };

    let (username, clientkey, area) = hue::pair_bridge(&bridge_ip, 45)?;
    Config::update(
        &output,
        Some(Config::new_default(
            &bridge_ip,
            &username,
            &clientkey,
            &area.configuration_id,
        )),
        |config| {
            config.bridge_ip = bridge_ip;
            config.username = username;
            config.clientkey = clientkey;
            config.entertainment_area_id = area.configuration_id.clone();
            config.entertainment_configuration_id = Some(area.configuration_id);
            config.hue_bridge_certificate_sha256 = Some(area.certificate_sha256);
            if !area.zones.is_empty() {
                config.zones = area.zones;
            }
            Ok(())
        },
    )?;
    println!(
        "\n[+] Configuration and Hue credentials successfully saved to {:?}",
        output
    );
    println!("    You can now run: lg-hue-sync run --config {:?}", output);
    Ok(())
}

pub(crate) async fn run_sync_hue(config_path: PathBuf, target_area: Option<String>) -> Result<()> {
    let mut config = Config::load(&config_path)?;
    let bridge_identity = (
        config.bridge_ip.clone(),
        config.username.clone(),
        config.hue_bridge_certificate_sha256.clone(),
    );
    info!(
        "Querying Hue Bridge at {} for Entertainment Areas...",
        config.bridge_ip
    );

    let area = hue::sync_entertainment_areas(
        &config.bridge_ip,
        &config.username,
        target_area.as_deref(),
        config.hue_bridge_certificate_sha256.as_deref(),
    )?;

    config = Config::update(&config_path, None, |config| {
        if (
            config.bridge_ip.as_str(),
            config.username.as_str(),
            config.hue_bridge_certificate_sha256.as_ref(),
        ) != (
            bridge_identity.0.as_str(),
            bridge_identity.1.as_str(),
            bridge_identity.2.as_ref(),
        ) {
            anyhow::bail!("Hue Bridge configuration changed during sync; retry");
        }
        config.entertainment_area_id = area.configuration_id.clone();
        config.entertainment_configuration_id = Some(area.configuration_id);
        config.hue_bridge_certificate_sha256 = Some(area.certificate_sha256);
        if !area.zones.is_empty() {
            config.zones = area.zones;
        }
        Ok(())
    })?;

    println!(
        "\n[+] Entertainment area '{}' (ID: {}) synced successfully!",
        area.name, config.entertainment_area_id
    );
    println!(
        "    Updated {} light zones in {:?}",
        config.zones.len(),
        config_path
    );
    println!("    3D light positions have been refreshed and projected to screen sampling boxes.");
    Ok(())
}

pub(crate) async fn run_pair_nanoleaf(ip_opt: Option<String>, config_path: PathBuf) -> Result<()> {
    let ip = match ip_opt {
        Some(ip) => ip,
        None => {
            use std::io::{stdin, stdout, Write};
            print!("Enter Nanoleaf 4D IP address: ");
            stdout().flush().ok();
            let mut line = String::new();
            stdin().read_line(&mut line)?;
            line.trim().to_string()
        }
    };

    let (auth_token, num_panels, panel_ids) = nanoleaf::pair_nanoleaf(&ip, 45)?;
    Config::update(
        &config_path,
        Some(Config::new_default("", "", "", "")),
        |config| {
            config.nanoleaf = Some(NanoleafConfig {
                enabled: true,
                ip: ip.clone(),
                auth_token,
                udp_port: 60222,
                segments: num_panels.max(30),
                panel_ids,
                alignment: NanoleafAlignment::default(),
            });
            Ok(())
        },
    )?;
    println!(
        "\n[+] Nanoleaf 4D controller at {} successfully paired and saved to {:?}",
        ip, config_path
    );
    println!(
        "    You can test it with: lg-hue-sync test-nanoleaf --config {:?}",
        config_path
    );
    println!(
        "    Run live sync with:   lg-hue-sync run --config {:?}",
        config_path
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing_invalid_config_is_never_replaced_with_defaults() {
        let path = std::env::temp_dir().join(format!(
            "lg-hue-sync-invalid-config-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, b"{invalid").unwrap();

        let result = Config::update(&path, Some(Config::new_default("", "", "", "")), |_| Ok(()));
        assert!(result.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"{invalid");

        std::fs::remove_file(&path).unwrap();
        let config = Config::update(&path, Some(Config::new_default("", "", "", "")), |_| Ok(()));
        assert!(config.is_ok());
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_file(format!("{}.lock", path.display())).unwrap();
    }
}
