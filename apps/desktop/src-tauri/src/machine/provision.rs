//! Creating and upgrading machines: everything that runs on the sandbox goes through
//! `provision.sh`, which is idempotent.

use crate::boat::Boat;
use crate::proxy::auth::random_token;
use crate::store::Machine;
use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use serde::Deserialize;

const PROVISION_SH: &str = include_str!("provision.sh");
const WATCHDOG_MJS: &str = include_str!("watchdog.mjs");
const T3_PORT: u16 = 3773;
const WATCHDOG_PORT: u16 = 3774;

/// Identifies the provisioning scripts; a machine provisioned with others gets them on wake.
pub fn revision() -> String {
    let hash = fnv1a(PROVISION_SH.bytes().chain(WATCHDOG_MJS.bytes()).chain(SLEEP_AFTER_SECS.to_string().bytes()));
    format!("{hash:016x}")
}

/// FNV-1a: stable across builds, unlike std's hasher.
pub fn fnv1a(bytes: impl IntoIterator<Item = u8>) -> u64 {
    bytes.into_iter().fold(0xcbf29ce484222325, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100000001b3))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Provisioned {
    pub environment_id: String,
    pub upstream_token: Option<String>,
}

/// Machine sizes, as Boat sells them.
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SizeInfo {
    pub id: &'static str,
    pub spec: &'static str,
    pub hourly: f64,
}

pub const SIZES: &[SizeInfo] = &[
    SizeInfo { id: "small", spec: "2 vCPU · 4 GB", hourly: 0.018 },
    SizeInfo { id: "default", spec: "4 vCPU · 8 GB", hourly: 0.036 },
    SizeInfo { id: "large", spec: "8 vCPU · 16 GB", hourly: 0.072 },
];

pub fn size(id: &str) -> Result<&'static SizeInfo> {
    SIZES.iter().find(|s| s.id == id).ok_or_else(|| anyhow!("unknown size {id}"))
}

/// How long a machine stays up after the last agent turn and the last action through t3boat.
/// Long enough to read a result and type a follow-up without waiting for a wake; idle minutes
/// cost under a cent even on the large size.
pub const SLEEP_AFTER_SECS: u64 = 300;

/// The machine's sandbox, as Boat lists it.
pub fn sandbox_name(name: &str) -> String {
    format!("t3boat · {name}")
}

/// The saved snapshot the watchdog replaces at every sleep. Boat deletes a stopped sandbox's
/// disk after 14 days; this stays, so the machine can be restored from it. Unique per machine,
/// whoever else uses the Boat account. Boat takes lowercase letters, digits and dashes.
pub fn snapshot_name(name: &str, machine_id: &str) -> String {
    let id: String = machine_id.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    format!("t3boat-{name}-{id}")
}

/// Copies the scripts over as a file (the watchdog secret must not appear in any command line)
/// and runs `provision.sh`.
pub async fn run(
    boat: &Boat,
    sandbox_id: &str,
    name: &str,
    t3_version: &str,
    watchdog_secret: &str,
    snapshot: &str,
    mint_upstream_token: bool,
) -> Result<Provisioned> {
    let b64 = |s: &str| base64::engine::general_purpose::STANDARD.encode(s);
    let script = format!(
        "set -e; d=$(mktemp -d); trap 'rm -rf \"$d\" \"$0\"' EXIT\n\
         echo {p} | base64 -d > \"$d/provision.sh\"; echo {w} | base64 -d > \"$d/watchdog.mjs\"\n\
         T3BOAT_SECRET={s} bash \"$d/provision.sh\" {v} {sleep} {snapshot} {mint} {name}\n",
        p = b64(PROVISION_SH),
        w = b64(WATCHDOG_MJS),
        v = shell_word(t3_version)?,
        s = shell_word(watchdog_secret)?,
        sleep = SLEEP_AFTER_SECS,
        snapshot = shell_word(snapshot)?,
        mint = u8::from(mint_upstream_token),
        name = shell_word(name)?,
    );
    let name = format!("t3boat-provision-{}.sh", random_token());
    let local = std::env::temp_dir().join(&name);
    crate::store::write_private(&local, script.as_bytes())?;
    let remote = format!("/tmp/{name}");
    let uploaded = boat.upload(sandbox_id, &local, &remote).await;
    let _ = std::fs::remove_file(&local);
    uploaded?;
    let out = boat.exec_ok(sandbox_id, &format!("bash {remote}"), 600).await?;
    let line = out
        .lines()
        .rev()
        .find(|l| l.starts_with('{'))
        .ok_or_else(|| anyhow!("provisioning printed no result"))?;
    Ok(serde_json::from_str(line)?)
}

/// Only `[A-Za-z0-9._-]` reaches the remote shell.
fn shell_word(s: &str) -> Result<&str> {
    if !s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')) {
        bail!("unsafe value for the remote shell: {s}");
    }
    Ok(s)
}

/// A saved snapshot's name or share code. It goes to `boat new --from` as an argument, so it must
/// not read as one of the CLI's options.
pub fn check_template(template: &str) -> Result<()> {
    let ok = template.len() <= 120
        && template.starts_with(|c: char| c.is_ascii_alphanumeric())
        && template.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if !ok {
        bail!("not a saved snapshot name: {template}");
    }
    Ok(())
}

/// Hosts the T3 server's and the watchdog's ports publicly: their URLs, in that order.
/// Idempotent: hosting again keeps the URLs.
pub async fn host(boat: &Boat, sandbox_id: &str) -> Result<(String, String)> {
    tokio::try_join!(boat.host_public(sandbox_id, T3_PORT), boat.host_public(sandbox_id, WATCHDOG_PORT))
}

/// Creates a new machine end to end. Takes about a minute, most of it Boat and the T3 install.
/// `template`: a saved Boat snapshot its disk starts from, with the user's own tools on it.
/// On failure after `created`, the caller deletes the sandbox.
#[allow(clippy::too_many_arguments)]
pub async fn create(
    boat: &Boat,
    name: &str,
    size: &str,
    t3_version: &str,
    local_port: u16,
    template: Option<&str>,
    created: impl FnOnce(&str),
    progress: impl Fn(&str),
) -> Result<Machine> {
    self::size(size)?;
    if let Some(t) = template {
        check_template(t)?;
    }
    progress("Creating sandbox on Boat");
    let id = boat
        .create(&sandbox_name(name), size, template)
        .await
        .context("creating the Boat sandbox")?;
    created(&id);
    progress("Installing T3 Code");
    let secret = random_token();
    let provisioned = run(boat, &id, name, t3_version, &secret, &snapshot_name(name, &id), true).await?;
    progress("Opening the machine's address");
    let (upstream_url, watchdog_url) = host(boat, &id).await?;
    Ok(Machine {
        id: id.clone(),
        sandbox_id: id,
        name: name.to_owned(),
        size: size.to_owned(),
        upstream_url,
        upstream_token: provisioned
            .upstream_token
            .ok_or_else(|| anyhow!("provisioning issued no token"))?,
        watchdog_url,
        watchdog_secret: secret,
        local_port,
        t3_version: t3_version.to_owned(),
        provision_revision: revision(),
        environment_id: provisioned.environment_id,
        client_tokens: Vec::new(),
        link_tokens: Vec::new(),
        created_at: chrono::Utc::now().timestamp(),
        next_due_at: None,
        connect: false,
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    /// A Boat CLI that answers like the real one and logs every call's arguments, one line each.
    fn fake_boat(dir: &std::path::Path) -> (Boat, PathBuf) {
        let log = dir.join("calls.log");
        let bin = dir.join("boat");
        let script = format!(
            r#"#!/bin/sh
echo "$*" >> '{log}'
case "$2" in
  new) printf '%s\n' '{{"id":"bx_fake"}}' ;;
  exec) printf '%s\n' '{{"exitCode":0,"stdout":"{{\"environmentId\":\"env-1\",\"upstreamToken\":\"tok\"}}","stderr":""}}' ;;
  host) printf '%s\n' '{{"url":"https://fake.boat.example"}}' ;;
esac
"#,
            log = log.display()
        );
        std::fs::write(&bin, script).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        (Boat::at(bin), log)
    }

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("t3boat-test-{}", random_token()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    async fn create_from(boat: &Boat, template: Option<&str>) -> Result<Machine> {
        create(boat, "dev", "small", "0.1.10", 47731, template, |_| {}, |_| {}).await
    }

    #[tokio::test]
    async fn a_machine_starts_from_the_chosen_template() {
        let dir = temp_dir();
        let (boat, log) = fake_boat(&dir);
        create_from(&boat, Some("worker-base")).await.unwrap();
        let calls = std::fs::read_to_string(&log).unwrap();
        let new = calls.lines().find(|l| l.starts_with("--json new ")).unwrap();
        assert!(new.ends_with("--from worker-base"), "{new}");

        std::fs::remove_file(&log).unwrap();
        create_from(&boat, None).await.unwrap();
        let calls = std::fs::read_to_string(&log).unwrap();
        assert!(!calls.contains("--from"), "{calls}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_template_cannot_pass_as_a_boat_option() {
        let dir = temp_dir();
        let (boat, log) = fake_boat(&dir);
        assert!(create_from(&boat, Some("--no-env")).await.is_err());
        assert!(!log.exists(), "boat was called");
        let _ = std::fs::remove_dir_all(dir);
    }
}
