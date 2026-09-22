use anyhow::{anyhow, Context, Result};
use openssl::ssl::{SslConnector, SslMethod, SslStream, SslVerifyMode};
use std::io::{Read, Write};
use std::net::UdpSocket;
use tracing::info;

#[derive(Debug)]
pub struct ConnectedUdpSocket(pub UdpSocket);

impl Read for ConnectedUdpSocket {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.recv(buf)
    }
}

impl Write for ConnectedUdpSocket {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.send(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub struct HueDtlsClient {
    stream: SslStream<ConnectedUdpSocket>,
}

impl HueDtlsClient {
    /// Connect to Hue Bridge DTLS port 2100 with PSK identity and client key
    pub fn connect(bridge_ip: &str, username: &str, clientkey_hex: &str) -> Result<Self> {
        let psk_bytes = hex_to_bytes(clientkey_hex).with_context(|| {
            "Failed to parse clientkey as hex (must be 32 hex chars / 16 bytes)"
        })?;

        info!(
            "Configuring DTLS 1.2 PSK connector for {}:2100...",
            bridge_ip
        );

        let mut builder = SslConnector::builder(SslMethod::dtls())
            .with_context(|| "Failed to create DTLS SSL connector builder")?;

        // Allow PSK ciphers supported by Hue Bridge
        builder
            .set_cipher_list("PSK-AES128-GCM-SHA256:PSK-AES256-GCM-SHA384")
            .with_context(|| "Failed to set PSK cipher list")?;

        // Disable server certificate verification since Hue uses pre-shared keys
        builder.set_verify(SslVerifyMode::NONE);

        let identity = username.to_string();
        builder.set_psk_client_callback(move |_ssl, _hint, identity_buf, psk_buf| {
            let id_bytes = identity.as_bytes();
            if id_bytes.len() > identity_buf.len() {
                return Err(openssl::error::ErrorStack::get());
            }
            identity_buf[..id_bytes.len()].copy_from_slice(id_bytes);

            if psk_bytes.len() > psk_buf.len() {
                return Err(openssl::error::ErrorStack::get());
            }
            psk_buf[..psk_bytes.len()].copy_from_slice(&psk_bytes);

            Ok(psk_bytes.len())
        });

        let connector = builder.build();

        // Bind local UDP socket and connect to Hue Bridge:2100
        let socket =
            UdpSocket::bind("0.0.0.0:0").with_context(|| "Failed to bind local UDP socket")?;
        let target_addr = format!("{}:2100", bridge_ip);
        socket
            .connect(&target_addr)
            .with_context(|| format!("Failed to connect UDP socket to {}", target_addr))?;
        socket
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .with_context(|| "Failed to set UDP socket read timeout")?;
        socket
            .set_write_timeout(Some(std::time::Duration::from_secs(5)))
            .with_context(|| "Failed to set UDP socket write timeout")?;

        let mut last_err = None;
        for attempt in 1..=4 {
            let socket_clone = socket
                .try_clone()
                .with_context(|| "Failed to clone UDP socket for DTLS retry")?;
            info!(
                "Performing DTLS handshake with {} (attempt {}/4)...",
                target_addr, attempt
            );
            match connector.connect("HueBridge", ConnectedUdpSocket(socket_clone)) {
                Ok(stream) => {
                    info!("[+] DTLS 1.2 handshake successful with Hue Bridge!");
                    return Ok(Self { stream });
                }
                Err(e) => {
                    let err_str = format!("{:?}", e);
                    tracing::warn!("DTLS handshake attempt {} failed: {}", attempt, err_str);
                    last_err = Some(e);
                    std::thread::sleep(std::time::Duration::from_millis(250));
                }
            }
        }

        Err(anyhow!(
            "DTLS handshake failed after 4 attempts: {:?}",
            last_err
        ))
    }

    /// Send a binary HueStream packet over the encrypted DTLS channel
    pub fn send(&mut self, packet: &[u8]) -> Result<()> {
        self.stream.write_all(packet)?;
        self.stream.flush()?;
        Ok(())
    }
}

fn hex_to_bytes(hex: &str) -> Result<Vec<u8>> {
    let hex = hex.trim();
    if !hex.len().is_multiple_of(2) {
        return Err(anyhow!("Hex string has odd length"));
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|e| anyhow!(e)))
        .collect()
}
