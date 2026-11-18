//! Thin wrapper over the user's own Boat CLI. Uses its existing login, or per machine a Boat API key
//! from the system keychain; never stores Boat credentials.

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;
use crate::process::command;

#[derive(Clone, Debug)]
pub struct Boat {
    bin: PathBuf,
    /// macOS's `security`, which reads keychain items.
    security: PathBuf,
    account: Option<BoatAccount>,
}

/// A Boat account other than the CLI's own login, chosen per machine. Holds where the key is,
/// never the key.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoatAccount {
    pub name: String,
    /// Keychain service of a generic password holding a Boat API key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keychain: Option<String>,
    /// Organization whose wallet the account's sandboxes bill to (`boat --org`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub org: Option<String>,
}

impl BoatAccount {
    /// Its values go to `security` and the Boat CLI as arguments and variables: none may read as
    /// an option.
    pub fn check(&self) -> Result<()> {
        let word = |s: &str| {
            !s.is_empty()
                && s.len() <= 120
                && s.starts_with(|c: char| c.is_ascii_alphanumeric())
                && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        };
        if !word(&self.name) {
            bail!("account names are letters, digits, dots, dashes or underscores");
        }
        if self.keychain.is_none() && self.org.is_none() {
            bail!("the account {} needs a keychain item, an organization or both", self.name);
        }
        for v in self.keychain.iter().chain(&self.org) {
            if !word(v) {
                bail!("not a keychain service or organization: {v}");
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Sandbox {
    pub id: String,
    pub state: String,
    pub stopped_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub identifier: Option<String>,
    pub signed_in: bool,
    pub plan: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecResult {
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub stdout: String,
    #[serde(default)]
    pub stderr: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub dollars: f64,
    pub seconds: u64,
}

/// A pending browser sign-in: the page that completes it, and the code that page may ask for.
#[derive(Clone, Debug)]
pub struct SignIn {
    pub url: String,
    pub code: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resume {
    Resumed,
    /// Boat deleted the stopped sandbox's disk: it keeps one 14 days.
    DataExpired,
}

#[derive(Clone, Debug, Default)]
pub struct Starts {
    pub used_today: u64,
    pub limit_today: u64,
}

impl Boat {
    /// Finds the CLI the Boat installer puts in `~/.ascii/bin`, else on PATH.
    pub fn locate() -> Option<Self> {
        let exe = if cfg!(windows) { "boat.exe" } else { "boat" };
        let installed = dirs::home_dir().map(|h| h.join(".ascii").join("bin").join(exe));
        installed
            .filter(|p| p.is_file())
            .or_else(|| which::which("boat").ok())
            .map(|bin| Self { bin, security: PathBuf::from("/usr/bin/security"), account: None })
    }

    #[cfg(test)]
    pub fn at(bin: PathBuf) -> Self {
        Self { bin, security: PathBuf::from("/usr/bin/security"), account: None }
    }

    #[cfg(test)]
    pub fn with_security(mut self, security: PathBuf) -> Self {
        self.security = security;
        self
    }

    /// The CLI acting as `account`; `None` is its own login.
    pub fn for_account(&self, account: Option<&BoatAccount>) -> Self {
        Self { account: account.cloned(), ..self.clone() }
    }

    pub fn boat_account(&self) -> Option<&BoatAccount> {
        self.account.as_ref()
    }

    /// A CLI command as this account. The key goes to the child's environment only: never into an
    /// argument, a file or this process's own environment.
    async fn command(&self) -> Result<tokio::process::Command> {
        let mut cmd = command(&self.bin);
        cmd.env("BOAT_NO_UPDATE", "1");
        let Some(account) = &self.account else { return Ok(cmd) };
        if let Some(org) = &account.org {
            cmd.env("BOAT_ORG", org);
        }
        if let Some(service) = &account.keychain {
            cmd.env("BOAT_API_KEY", self.keychain_secret(service).await?);
        }
        Ok(cmd)
    }

    async fn keychain_secret(&self, service: &str) -> Result<String> {
        if !cfg!(unix) {
            bail!("Boat keys from the keychain work on macOS only");
        }
        let out = command(&self.security)
            .args(["find-generic-password", "-s", service, "-w"])
            .kill_on_drop(true)
            .output();
        let out = tokio::time::timeout(Duration::from_secs(20), out)
            .await
            .map_err(|_| anyhow!("reading {service} from the keychain timed out"))?
            .with_context(|| format!("reading {service} from the keychain"))?;
        let key = String::from_utf8(out.stdout).unwrap_or_default().trim().to_owned();
        if !out.status.success() || key.is_empty() {
            bail!("no Boat API key in the keychain under {service}");
        }
        Ok(key)
    }

    /// Runs the CLI; returns whether it succeeded, stdout and stderr.
    async fn output(&self, args: &[&str], timeout: Duration) -> Result<(bool, String, String)> {
        let mut cmd = self.command().await?;
        // `--json` first: anything after `--` belongs to the remote command, and a nested
        // subcommand (`snapshot rm`) is only read as one with no option before it.
        cmd.arg("--json").args(args).kill_on_drop(true);
        // Only the subcommand goes into errors: arguments can hold scripts.
        let sub = args.first().copied().unwrap_or_default();
        let out = tokio::time::timeout(timeout, cmd.output())
            .await
            .map_err(|_| anyhow!("boat {sub} timed out"))?
            .with_context(|| format!("running boat {sub}"))?;
        Ok((
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        ))
    }

    async fn run(&self, args: &[&str], timeout: Duration) -> Result<String> {
        let (ok, stdout, stderr) = self.output(args, timeout).await?;
        if !ok {
            let sub = args.first().copied().unwrap_or_default();
            bail!("boat {sub} failed: {}", said(&stdout, &stderr));
        }
        Ok(stdout)
    }

    /// The CLI prints JSON lines for long operations; the last line is the result.
    fn last_json(out: &str) -> Result<Value> {
        let line = out
            .lines()
            .rev()
            .find(|l| l.trim_start().starts_with('{'))
            .ok_or_else(|| anyhow!("boat printed no JSON: {out}"))?;
        Ok(serde_json::from_str(line)?)
    }

    pub async fn account(&self) -> Result<Account> {
        let v = Self::last_json(&self.run(&["status"], Duration::from_secs(20)).await?)?;
        Ok(serde_json::from_value(v["account"].clone())?)
    }

    /// Returns the new sandbox's id. `snapshot`: the saved snapshot its disk starts from.
    pub async fn create(&self, name: &str, size: &str, snapshot: Option<&str>) -> Result<String> {
        let mut args = vec!["new", "--name", name, "--type", size, "--no-auto-stop"];
        args.extend(snapshot.into_iter().flat_map(|s| ["--from", s]));
        let out = self.run(&args, Duration::from_secs(300)).await?;
        let v = Self::last_json(&out)?;
        let id = v["id"].as_str().ok_or_else(|| anyhow!("boat new returned no id"))?;
        Ok(id.to_owned())
    }

    pub async fn info(&self, id: &str) -> Result<Sandbox> {
        let v = Self::last_json(&self.run(&["info", id], Duration::from_secs(20)).await?)?;
        Ok(serde_json::from_value(v["sandbox"].clone())?)
    }

    pub async fn list(&self) -> Result<Vec<Sandbox>> {
        let v = Self::last_json(&self.run(&["list", "--all"], Duration::from_secs(20)).await?)?;
        Ok(serde_json::from_value(v["sandboxes"].clone())?)
    }

    /// Gone only when Boat's own list confirms it: one bad answer must not retire a machine.
    pub async fn confirmed_gone(&self, id: &str) -> bool {
        matches!(self.list().await, Ok(l) if l.iter().all(|s| s.id != id))
    }

    /// Resumes onto `size`, which is how a machine changes size.
    pub async fn resume(&self, id: &str, size: &str) -> Result<Resume> {
        let (ok, stdout, stderr) =
            self.output(&["resume", id, "--no-auto-stop", "--type", size], Duration::from_secs(300)).await?;
        let said = said(&stdout, &stderr);
        match ok {
            true => Ok(Resume::Resumed),
            false if said.contains("snapshot_data_expired") => Ok(Resume::DataExpired),
            false => bail!("boat resume failed: {said}"),
        }
    }

    pub async fn delete_snapshot(&self, name: &str) -> Result<()> {
        self.run(&["snapshot", "rm", name], Duration::from_secs(60)).await.map(drop)
    }

    pub async fn delete(&self, id: &str) -> Result<()> {
        self.run(&["delete", id, "--yes"], Duration::from_secs(120))
            .await
            .map(drop)
    }

    /// Exposes `port` (bound to loopback on the sandbox) on a stable public HTTPS URL.
    pub async fn host_public(&self, id: &str, port: u16) -> Result<String> {
        let port = port.to_string();
        let v = Self::last_json(
            &self
                .run(&["host", id, &port, "--public", "--localhost"], Duration::from_secs(60))
                .await?,
        )?;
        v["url"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| anyhow!("boat host returned no url"))
    }

    pub async fn exec(&self, id: &str, script: &str, timeout_secs: u64) -> Result<ExecResult> {
        let t = timeout_secs.to_string();
        // The CLI exits with the command's code; its JSON result is what matters.
        let (_, stdout, stderr) = self
            .output(
                &["exec", id, "--timeout", &t, "--", "bash", "-lc", script],
                Duration::from_secs(timeout_secs + 60),
            )
            .await?;
        match Self::last_json(&stdout) {
            Ok(v) => Ok(serde_json::from_value(v)?),
            Err(_) => bail!("boat exec failed: {}", stderr.trim()),
        }
    }

    /// Runs `script` and fails unless it exits 0. Returns stdout.
    pub async fn exec_ok(&self, id: &str, script: &str, timeout_secs: u64) -> Result<String> {
        let r = self.exec(id, script, timeout_secs).await?;
        if r.exit_code != Some(0) {
            let detail = if r.stderr.trim().is_empty() { r.stdout.trim() } else { r.stderr.trim() };
            bail!("command on the machine failed ({:?}): {detail}", r.exit_code);
        }
        Ok(r.stdout)
    }

    pub async fn usage_since(&self, id: &str, since_unix: i64) -> Result<Usage> {
        let since = since_unix.to_string();
        let v = Self::last_json(
            &self.run(&["usage", id, "--since", &since], Duration::from_secs(20)).await?,
        )?;
        Ok(serde_json::from_value(v)?)
    }

    pub async fn starts(&self) -> Result<Starts> {
        let v = Self::last_json(&self.run(&["limits", "--all"], Duration::from_secs(20)).await?)?;
        let day = &v["starts"]["day"];
        Ok(Starts {
            used_today: day["used"].as_u64().unwrap_or(0),
            limit_today: day["limit"].as_u64().unwrap_or(0),
        })
    }

    /// Starts a browser sign-in, or picks up the one already pending. Without a terminal the
    /// CLI opens no browser, so the caller opens the page.
    pub async fn start_sign_in(&self) -> Result<SignIn> {
        let v = Self::last_json(&self.run(&["onboard"], Duration::from_secs(60)).await?)?;
        let url = v["url"].as_str().ok_or_else(|| anyhow!("boat onboard gave no sign-in page: {v}"))?;
        Ok(SignIn { url: url.to_owned(), code: v["userCode"].as_str().map(str::to_owned) })
    }

    /// Waits until the started sign-in completes or expires.
    pub async fn finish_sign_in(&self) -> Result<()> {
        self.run(&["onboard", "--wait"], Duration::from_secs(900)).await.map(drop)
    }

    /// Copies a local file to `remote_path` on the sandbox.
    pub async fn upload(&self, id: &str, local: &Path, remote_path: &str) -> Result<()> {
        let to = format!("{id}:{remote_path}");
        self.run(&["scp", path_str(local)?, &to], Duration::from_secs(1800)).await.map(drop)
    }

    /// Forwards `port` on this computer to the same port in the sandbox, for as long as the
    /// returned process lives.
    pub async fn forward(&self, id: &str, port: u16) -> Result<tokio::process::Child> {
        // Else another listener on the port would pass the readiness check below.
        if std::net::TcpListener::bind(("127.0.0.1", port)).is_err() {
            bail!("port {port} is in use on this computer; finish or close the other sign-in first");
        }
        let p = port.to_string();
        let mut child = self
            .command()
            .await?
            .args(["forward", id, "--remote", &p, "--local", &p])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .context("running boat forward")?;
        for _ in 0..100 {
            if let Some(status) = child.try_wait()? {
                bail!("boat forward exited ({status}); is port {port} free on this computer?");
            }
            if tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
                return Ok(child);
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        bail!("boat forward did not open port {port}")
    }
}

/// What a failed command said: its stderr, else its stdout.
fn said<'a>(stdout: &'a str, stderr: &'a str) -> &'a str {
    if stderr.trim().is_empty() { stdout.trim() } else { stderr.trim() }
}

/// Command-line form of a path; the Boat CLI and git only take UTF-8 here.
pub(crate) fn path_str(path: &Path) -> Result<&str> {
    path.to_str().ok_or_else(|| anyhow!("path is not UTF-8: {}", path.display()))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn script(path: &Path, body: &str) {
        std::fs::write(path, format!("#!/bin/sh\n{body}")).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[tokio::test]
    async fn a_machine_account_reaches_boat_only_through_its_environment() {
        let dir = std::env::temp_dir().join(format!("t3boat-boat-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("calls.log");
        script(
            &dir.join("security"),
            &format!("echo \"security $*\" >> '{}'; echo team-key-123", log.display()),
        );
        script(
            &dir.join("boat"),
            &format!(
                "echo \"boat $* key=$BOAT_API_KEY org=$BOAT_ORG\" >> '{}'; echo '{{\"sandboxes\":[]}}'",
                log.display()
            ),
        );
        let account = BoatAccount {
            name: "team".into(),
            keychain: Some("boat-api-key-team".into()),
            org: Some("team-org".into()),
        };
        let boat = Boat::at(dir.join("boat")).with_security(dir.join("security")).for_account(Some(&account));
        boat.list().await.unwrap();
        let calls = std::fs::read_to_string(&log).unwrap();
        assert!(calls.contains("security find-generic-password -s boat-api-key-team -w"), "{calls}");
        let call = calls.lines().find(|l| l.starts_with("boat ")).unwrap();
        assert!(call.ends_with("key=team-key-123 org=team-org"), "{call}");
        assert!(!call.trim_end_matches("key=team-key-123 org=team-org").contains("team-key-123"), "key on the command line: {call}");

        std::fs::remove_file(&log).unwrap();
        script(&dir.join("security"), "echo 'security: item not found' >&2; exit 44");
        let err = boat.list().await.unwrap_err().to_string();
        assert!(err.contains("boat-api-key-team"), "{err}");
        assert!(!log.exists(), "boat ran without its account's key");
        let _ = std::fs::remove_dir_all(dir);
    }
}
