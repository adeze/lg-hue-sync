use anyhow::{anyhow, ensure, Context, Result};
use serde_json::Value;
use std::{path::Path, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, BufReader},
    process::Command,
    time::timeout,
};
use tokio_util::sync::CancellationToken;

const MAX_REPLY: u64 = 64 * 1024;

pub async fn query(uri: &str, payload: &str) -> Result<Value> {
    query_program(
        Path::new("/usr/bin/luna-send"),
        uri,
        payload,
        Duration::from_millis(1500),
    )
    .await
}

pub async fn subscribe(
    uri: &str,
    payload: &str,
    shutdown: CancellationToken,
    on_event: impl FnMut(Value),
) -> Result<()> {
    subscribe_program(
        Path::new("/usr/bin/luna-send"),
        uri,
        payload,
        shutdown,
        Duration::from_millis(1500),
        on_event,
    )
    .await
}

async fn subscribe_program(
    program: &Path,
    uri: &str,
    payload: &str,
    shutdown: CancellationToken,
    initial_deadline: Duration,
    mut on_event: impl FnMut(Value),
) -> Result<()> {
    if shutdown.is_cancelled() {
        return Ok(());
    }
    let mut child = Command::new(program)
        .kill_on_drop(true)
        .args(["-i", uri, payload])
        // Keep stdin open as for one-shot queries; omit -f for one JSON reply per line.
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("could not start webOS subscription")?;
    let result = async {
        let mut reader = BufReader::new(
            child
                .stdout
                .take()
                .context("webOS subscription output unavailable")?,
        );
        let mut initial = true;
        loop {
            let value = if initial {
                tokio::select! {
                    biased;
                    _ = shutdown.cancelled() => return Ok(()),
                    reply = timeout(initial_deadline, subscription_reply(&mut reader)) => {
                        reply.map_err(|_| anyhow!("webOS subscription acknowledgement timed out"))??
                    }
                }
            } else {
                // Unchanged sources may remain silent indefinitely; only the first reply has a deadline.
                tokio::select! {
                    biased;
                    _ = shutdown.cancelled() => return Ok(()),
                    reply = subscription_reply(&mut reader) => reply?,
                }
            };
            ensure!(
                value.is_object() && value["returnValue"] == true,
                "webOS subscription refused or missing acknowledgement"
            );
            ensure!(
                value.get("subscribed") != Some(&Value::Bool(false)),
                "webOS subscription was not accepted"
            );
            on_event(value);
            initial = false;
        }
    }
    .await;
    // Reap on cancellation, malformed replies and EOF, including children that close stdout early.
    let _ = child.start_kill();
    timeout(Duration::from_secs(1), child.wait())
        .await
        .map_err(|_| anyhow!("webOS subscription cleanup timed out"))?
        .context("could not reap webOS subscription")?;
    result
}

async fn subscription_reply(reader: &mut (impl AsyncBufRead + Unpin)) -> Result<Value> {
    let mut bytes = Vec::with_capacity(4096);
    reader
        .take(MAX_REPLY + 1)
        .read_until(b'\n', &mut bytes)
        .await
        .context("could not read webOS subscription")?;
    ensure!(
        bytes.len() as u64 <= MAX_REPLY,
        "webOS subscription response too large"
    );
    ensure!(
        bytes.last() == Some(&b'\n'),
        "webOS subscription ended before a complete reply"
    );
    serde_json::from_slice(&bytes).map_err(|_| anyhow!("webOS subscription returned invalid JSON"))
}

async fn query_program(
    program: &Path,
    uri: &str,
    payload: &str,
    deadline: Duration,
) -> Result<Value> {
    let mut child = Command::new(program)
        .kill_on_drop(true)
        .args(["-n", "1", "-f", "-w", "1000", uri, payload])
        // This firmware exits at stdin EOF before its asynchronous reply arrives.
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("could not start webOS query")?;
    let result = timeout(deadline, async {
        let mut bytes = Vec::new();
        child
            .stdout
            .take()
            .context("webOS query output unavailable")?
            .take(MAX_REPLY + 1)
            .read_to_end(&mut bytes)
            .await
            .context("could not read webOS query")?;
        ensure!(
            bytes.len() as u64 <= MAX_REPLY,
            "webOS query response too large"
        );
        ensure!(
            child
                .wait()
                .await
                .context("could not wait for webOS query")?
                .success(),
            "webOS query process failed"
        );
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|_| anyhow!("webOS query returned invalid JSON"))?;
        ensure!(
            value.is_object() && value["returnValue"] == true,
            "webOS query refused or missing acknowledgement"
        );
        Ok(value)
    })
    .await
    .unwrap_or_else(|_| Err(anyhow!("webOS query timed out")));
    if result.is_err() {
        let _ = child.kill().await;
    }
    result
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        path::PathBuf,
        sync::atomic::{AtomicUsize, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    struct SubscriptionFixture(PathBuf);

    impl SubscriptionFixture {
        fn new() -> Self {
            static NEXT_FIXTURE: AtomicUsize = AtomicUsize::new(0);
            let directory = std::env::temp_dir().join(format!(
                "lg-luna-subscription-{}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&directory).unwrap();
            let fixture = Self(directory);
            fs::write(
                fixture.program(),
                r#"#!/bin/sh
[ -p /dev/stdin ] || exit 98
[ "$1" = '-i' ] && [ "$2" = 'luna://test/get' ] && [ "$3" = '{"subscribe":true}' ] || exit 99
echo $$ > "$0.pid"
# An EOF marker would appear immediately if the parent closed its pipe writer.
(read -r ignored; : > "$0.eof") <&0 &
reader=$!
sleep 0.05
kill "$reader" 2>/dev/null
wait "$reader" 2>/dev/null
[ ! -e "$0.eof" ] || exit 97
case "$(cat "$0.mode")" in
events) printf '%s\n' '{"returnValue":true,"subscribed":true,"value":1}' '{"returnValue":true,"value":2}' ;;
idle) printf '%s\n' '{"returnValue":true,"subscribed":true}' ;;
partial_after_ack) printf '%s\n' '{"returnValue":true}'; printf '{"returnValue":' ;;
partial) printf '{"returnValue":' ;;
large) dd if=/dev/zero bs=65537 count=1 2>/dev/null ;;
refused) printf '%s\n' '{"returnValue":false,"errorText":"private-error"}' ;;
unsubscribed) printf '%s\n' '{"returnValue":true,"subscribed":false}' ;;
unacknowledged) printf '%s\n' '{"private-field":true}' ;;
malformed) printf '%s\n' 'private-invalid-json' ;;
failed) exit 2 ;;
eof) exit 0 ;;
wait) ;;
esac
exec sleep 30
"#,
            )
            .unwrap();
            fs::set_permissions(fixture.program(), fs::Permissions::from_mode(0o700)).unwrap();
            fixture
        }

        fn program(&self) -> PathBuf {
            self.0.join("luna-send")
        }

        fn mode(&self, mode: &str) {
            let _ = fs::remove_file(self.0.join("luna-send.pid"));
            let _ = fs::remove_file(self.0.join("luna-send.eof"));
            fs::write(self.0.join("luna-send.mode"), mode).unwrap();
        }

        fn assert_reaped(&self) {
            let pid = fs::read_to_string(self.0.join("luna-send.pid")).unwrap();
            assert!(!std::process::Command::new("/bin/kill")
                .args(["-0", pid.trim()])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .unwrap()
                .success());
        }
    }

    impl Drop for SubscriptionFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[tokio::test]
    async fn subscription_delivers_multiple_compact_replies_and_reaps_on_cancellation() {
        let fixture = SubscriptionFixture::new();
        fixture.mode("events");
        let shutdown = CancellationToken::new();
        let mut values = Vec::new();
        subscribe_program(
            &fixture.program(),
            "luna://test/get",
            r#"{"subscribe":true}"#,
            shutdown.clone(),
            Duration::from_secs(1),
            |value| {
                values.push(value["value"].as_u64().unwrap());
                if values.len() == 2 {
                    shutdown.cancel();
                }
            },
        )
        .await
        .unwrap();
        assert_eq!(values, vec![1, 2]);
        fixture.assert_reaped();
    }

    #[tokio::test]
    async fn subscription_bounds_partial_large_and_failed_replies_without_exposing_payloads() {
        let fixture = SubscriptionFixture::new();
        for mode in [
            "partial",
            "large",
            "refused",
            "unsubscribed",
            "unacknowledged",
            "malformed",
            "failed",
            "eof",
        ] {
            fixture.mode(mode);
            let mut events = 0;
            let result = subscribe_program(
                &fixture.program(),
                "luna://test/get",
                r#"{"subscribe":true}"#,
                CancellationToken::new(),
                Duration::from_secs(1),
                |_| events += 1,
            )
            .await;
            let error = result.unwrap_err().to_string();
            assert!(!error.contains("private"));
            assert_eq!(events, 0);
            fixture.assert_reaped();
        }
    }

    #[tokio::test]
    async fn subscription_idle_has_no_reply_deadline_and_cancellation_interrupts_pending_reads() {
        let fixture = SubscriptionFixture::new();
        for mode in ["wait", "idle", "partial_after_ack"] {
            fixture.mode(mode);
            let shutdown = CancellationToken::new();
            let program = fixture.program();
            let mut events = 0;
            let subscription = subscribe_program(
                &program,
                "luna://test/get",
                r#"{"subscribe":true}"#,
                shutdown.clone(),
                if mode == "wait" {
                    Duration::from_millis(1500)
                } else {
                    Duration::from_millis(500)
                },
                |_| events += 1,
            );
            tokio::pin!(subscription);
            let observation = if mode == "wait" {
                Duration::from_millis(500)
            } else {
                Duration::from_millis(750)
            };
            assert!(timeout(observation, &mut subscription).await.is_err());
            shutdown.cancel();
            subscription.await.unwrap();
            fixture.assert_reaped();
        }
    }

    #[tokio::test]
    async fn query_holds_stdin_open_bounds_replies_and_reaps_timed_out_children() {
        struct Fixture(PathBuf);
        impl Drop for Fixture {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let fixture = Fixture(std::env::temp_dir().join(format!(
                "lg-luna-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            )));
        fs::create_dir(&fixture.0).unwrap();
        let program = fixture.0.join("luna-send");
        fs::write(
            &program,
            r#"#!/bin/sh
[ -p /dev/stdin ] || exit 98
[ "$6" = 'luna://com.webos.applicationManager/getForegroundAppInfo' ] && [ "$7" = '{}' ] || exit 99
case "$(cat "$0.mode")" in
good) printf '{"returnValue":true,"appId":"com.webos.app.hdmi2"}' ;;
failed) exit 2 ;;
malformed) printf 'private-invalid-payload' ;;
refused) printf '{"returnValue":false,"errorText":"private-error"}' ;;
unacknowledged) printf '{"appId":"com.webos.app.hdmi2"}' ;;
large) dd if=/dev/zero bs=65537 count=1 2>/dev/null ;;
wait) echo $$ > "$0.pid"; exec sleep 30 ;;
esac
"#,
        )
        .unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
        for mode in [
            "good",
            "failed",
            "malformed",
            "refused",
            "unacknowledged",
            "large",
            "wait",
        ] {
            fs::write(fixture.0.join("luna-send.mode"), mode).unwrap();
            let reply = query_program(
                &program,
                "luna://com.webos.applicationManager/getForegroundAppInfo",
                "{}",
                Duration::from_millis(500),
            )
            .await;
            if mode == "good" {
                assert_eq!(reply.unwrap()["appId"], "com.webos.app.hdmi2");
            } else {
                let error = reply.unwrap_err().to_string();
                assert!(!error.contains("private"));
            }
        }
        let pid = fs::read_to_string(fixture.0.join("luna-send.pid")).unwrap();
        assert!(!std::process::Command::new("/bin/kill")
            .args(["-0", pid.trim()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success());
        assert!(query_program(
            &fixture.0.join("missing"),
            "luna://unused",
            "{}",
            Duration::from_millis(500)
        )
        .await
        .is_err());
    }
}
