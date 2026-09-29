use anyhow::{anyhow, Error};
use std::time::Duration;

/// LAN controllers must not inherit a host proxy or redirect credentials elsewhere.
pub(crate) fn agent(timeout: Option<Duration>) -> ureq::Agent {
    ureq::Agent::config_builder()
        .proxy(None)
        .max_redirects(0)
        .timeout_global(timeout)
        .build()
        .into()
}

/// Do not retain a token-bearing request URL through an HTTP error chain.
pub(crate) fn request_error(operation: &str, error: ureq::Error) -> Error {
    match error {
        ureq::Error::StatusCode(status) => anyhow!("{operation} failed (HTTP {status})"),
        ureq::Error::Timeout(_) => anyhow!("{operation} timed out"),
        _ => anyhow!("{operation} failed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lan_requests_ignore_environment_proxies_and_redirects() {
        if std::env::var_os("LG_HUE_SYNC_PROXY_TEST_CHILD").is_none() {
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "local_http::tests::lan_requests_ignore_environment_proxies_and_redirects",
                ])
                .env("LG_HUE_SYNC_PROXY_TEST_CHILD", "1")
                .env("HTTP_PROXY", "http://127.0.0.1:9")
                .env("HTTPS_PROXY", "http://127.0.0.1:9")
                .env("ALL_PROXY", "http://127.0.0.1:9")
                .status()
                .unwrap();
            assert!(status.success());
            return;
        }
        let agent = agent(None);
        assert!(agent.config().proxy().is_none());
        assert_eq!(agent.config().max_redirects(), 0);
    }

    #[test]
    fn request_errors_never_include_a_token_bearing_url() {
        let token_url = "http://controller/api/v1/private-token/effects";
        let error = request_error("Nanoleaf control", ureq::Error::BadUri(token_url.into()));
        assert!(!error.to_string().contains("private-token"));
        assert_eq!(error.to_string(), "Nanoleaf control failed");
    }
}
