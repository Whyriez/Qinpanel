//! Read-only fixed-target inspection. No network connections or configuration writes.
use panel_proto::{ConnectivityInspection, ConnectivityTarget, LocalCheck};
use std::{process::Stdio, time::Duration};
use tokio::io::AsyncReadExt;

async fn command(binary: &str, args: &[&str]) -> Option<(bool, Vec<u8>)> {
    tokio::time::timeout(Duration::from_secs(2), async {
        let mut child = tokio::process::Command::new(binary)
            .args(args)
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .stdout(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .ok()?;
        let mut bytes = Vec::new();
        child
            .stdout
            .take()?
            .take(4097)
            .read_to_end(&mut bytes)
            .await
            .ok()?;
        if bytes.len() > 4096 {
            return None;
        }
        Some((child.wait().await.ok()?.success(), bytes))
    })
    .await
    .ok()
    .flatten()
}
pub fn service_state(output: Option<&[u8]>) -> (LocalCheck, LocalCheck) {
    let Some(output) = output.and_then(|s| std::str::from_utf8(s).ok()) else {
        return (LocalCheck::Unknown, LocalCheck::Unknown);
    };
    let installed = if output.lines().any(|s| s == "LoadState=loaded") {
        LocalCheck::Pass
    } else if output.lines().any(|s| s == "LoadState=not-found") {
        LocalCheck::Fail
    } else {
        LocalCheck::Unknown
    };
    // PostgreSQL's umbrella unit can be active(exited): never call it running.
    let running = if output.lines().any(|s| s == "ActiveState=active")
        && output.lines().any(|s| s == "SubState=running")
    {
        LocalCheck::Pass
    } else if output
        .lines()
        .any(|s| matches!(s, "ActiveState=inactive" | "ActiveState=failed"))
    {
        LocalCheck::Fail
    } else {
        LocalCheck::Unknown
    };
    (installed, running)
}
pub fn listener_state(output: Option<&[u8]>, port: u16, ipv6: bool) -> LocalCheck {
    let Some(output) = output.and_then(|s| std::str::from_utf8(s).ok()) else {
        return LocalCheck::Unknown;
    };
    let mut rows = output.lines();
    if !rows.next().is_some_and(|s| s.contains("local_address")) {
        return LocalCheck::Unknown;
    }
    for line in rows {
        let fields: Vec<_> = line.split_whitespace().collect();
        let Some((address, value)) = fields.get(1).and_then(|s| s.split_once(':')) else {
            return LocalCheck::Unknown;
        };
        if address.len() != if ipv6 { 32 } else { 8 }
            || !address.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return LocalCheck::Unknown;
        }
        let Ok(value) = u16::from_str_radix(value, 16) else {
            return LocalCheck::Unknown;
        };
        let Some(state) = fields
            .get(3)
            .filter(|s| s.len() == 2 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        else {
            return LocalCheck::Unknown;
        };
        if *state == "0A" && value == port {
            return LocalCheck::Pass;
        }
    }
    LocalCheck::Fail
}
async fn read_proc(path: &'static str) -> Option<Vec<u8>> {
    tokio::time::timeout(Duration::from_secs(2), async {
        let file = tokio::fs::File::open(path).await.ok()?;
        let mut bytes = Vec::new();
        file.take(1_048_577).read_to_end(&mut bytes).await.ok()?;
        (bytes.len() <= 1_048_576).then_some(bytes)
    })
    .await
    .ok()
    .flatten()
}
pub async fn inspect(target: ConnectivityTarget) -> ConnectivityInspection {
    let (unit, port) = target.local();
    let args = [
        "show",
        "--no-pager",
        "--property=LoadState,ActiveState,SubState",
        unit,
    ];
    let (unit_state, ipv4, ipv6, guard) = tokio::join!(
        command("systemctl", &args),
        read_proc("/proc/net/tcp"),
        read_proc("/proc/net/tcp6"),
        command("nft", &["list", "table", "inet", "panel_loopback_guard"])
    );
    // systemctl show may exit nonzero for a missing unit while still returning
    // the explicit LoadState=not-found properties. Preserve those safe fields.
    let (installed, running) =
        service_state(unit_state.as_ref().map(|(_, bytes)| bytes.as_slice()));
    ConnectivityInspection {
        installed,
        running,
        listener_ipv4: listener_state(ipv4.as_deref(), port, false),
        listener_ipv6: listener_state(ipv6.as_deref(), port, true),
        panel_guard_present: if guard.is_some_and(|(success, _)| success) {
            LocalCheck::Pass
        } else {
            LocalCheck::Unknown
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn service_requires_running_substate_and_preserves_unknown() {
        assert_eq!(
            service_state(Some(
                b"LoadState=loaded\nActiveState=active\nSubState=running\n"
            )),
            (LocalCheck::Pass, LocalCheck::Pass)
        );
        assert_eq!(
            service_state(Some(
                b"LoadState=loaded\nActiveState=active\nSubState=exited\n"
            )),
            (LocalCheck::Pass, LocalCheck::Unknown)
        );
        assert_eq!(
            service_state(Some(b"LoadState=not-found\nActiveState=inactive\n")),
            (LocalCheck::Fail, LocalCheck::Fail)
        );
        assert_eq!(
            service_state(None),
            (LocalCheck::Unknown, LocalCheck::Unknown)
        );
    }
    #[test]
    fn listener_matches_only_listen_state_exact_port_and_family() {
        let v4 = b"sl local_address rem_address st\n0: 0100007F:0016 00000000:0000 0A\n";
        assert_eq!(listener_state(Some(v4), 22, false), LocalCheck::Pass);
        assert_eq!(listener_state(Some(v4), 2222, false), LocalCheck::Fail);
        assert_eq!(listener_state(Some(v4), 22, true), LocalCheck::Unknown);
        assert_eq!(listener_state(None, 22, false), LocalCheck::Unknown);
        assert_eq!(listener_state(Some(b"bad"), 22, false), LocalCheck::Unknown);
        assert_eq!(
            listener_state(
                Some(b"sl local_address rem_address st\n0: 00000000:0016 00000000:0000 01\n"),
                22,
                false
            ),
            LocalCheck::Fail
        );
    }
    #[test]
    fn local_targets_are_closed_and_never_internal_api_port() {
        use ConnectivityTarget::*;
        for target in [
            Panel,
            WebsiteHttp,
            WebsiteHttps,
            Ssh,
            MariaDb,
            PostgreSql,
            Imap,
            Submission,
            SmtpInbound,
        ] {
            assert_ne!(target.local().1, 7700);
        }
        assert!(
            serde_json::from_str::<ConnectivityTarget>(r#"{"host":"127.0.0.1","port":7700}"#)
                .is_err()
        );
    }
}
