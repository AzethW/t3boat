//! Signing an agent in on a machine with the vendor's own CLI login. The login runs on the
//! machine and its localhost callback port is forwarded to this computer, so the browser's
//! redirect after "Authorize" lands on the machine: no code to paste, and the credential is
//! created on the machine and stays there. This computer's own logins are never touched.
//! t3boat-server's browser is on another computer, so there the login runs with no callback
//! instead (`start_manual_sign_in`).

use super::MachineRuntime;
use crate::proxy::upstream;
use anyhow::{anyhow, bail, Context, Result};
use std::time::{Duration, Instant};

const URL_TIMEOUT: Duration = Duration::from_secs(30);
const LOGIN_TIMEOUT: Duration = Duration::from_secs(600);

#[derive(Clone, Copy, Debug)]
pub enum Agent {
    Claude,
    Codex,
}

impl Agent {
    /// By the provider's display name, as `signedOut` lists it.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "Claude" => Some(Self::Claude),
            "Codex" => Some(Self::Codex),
            _ => None,
        }
    }

    fn login(self) -> &'static str {
        match self {
            Self::Claude => "claude auth login",
            Self::Codex => "codex login",
        }
    }

    /// The login with no local callback, for a browser on another computer.
    fn manual_login(self) -> &'static str {
        match self {
            Self::Claude => "claude auth login",
            Self::Codex => "codex login --device-auth",
        }
    }

    /// Exits 0 once signed in.
    fn status(self) -> &'static str {
        match self {
            Self::Claude => "claude auth status >/dev/null 2>&1",
            Self::Codex => "codex login status >/dev/null 2>&1",
        }
    }
}

/// A shell command that ends any running `login` (a command line such as `claude auth login`).
/// The script running it holds the same words, so a plain pattern would end the script too: the
/// first letter goes in brackets, which matches the login but not this command's own text.
pub fn stop_login(login: &str) -> String {
    let mut chars = login.chars();
    let pattern = match chars.next() {
        Some(first) => format!("[{first}]{}", chars.as_str()),
        None => String::new(),
    };
    format!("pkill -f '{pattern}'")
}

#[cfg(all(test, unix))]
mod stop_login_tests {
    use super::stop_login;
    use std::process::{Command, Stdio};

    /// The command runs inside `bash -lc "<script>"`, as `boat exec` runs it, so its own command
    /// line holds the login's words too. It must end the login and carry on.
    #[test]
    fn ends_the_login_but_not_the_script_running_it() {
        let login = "t3boat-test-login auth login";
        // A stand-in for the vendor's login: a process whose command line holds `login`.
        let mut decoy = Command::new("bash")
            .args(["-c", &format!("exec -a '{login}' sleep 60")])
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(200));
        let script = format!("{} 2>/dev/null; echo survived", stop_login(login));
        let out = Command::new("bash").args(["-c", &script]).output().unwrap();
        let ended = decoy.try_wait().unwrap().is_some() || {
            std::thread::sleep(std::time::Duration::from_millis(300));
            decoy.try_wait().unwrap().is_some()
        };
        let _ = decoy.kill();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "survived", "the script killed itself: {:?}", out.status);
        assert!(out.status.success(), "{:?}", out.status);
        assert!(ended, "the login is still running");
    }
}

/// Where a manual sign-in keeps its log and stdin on the machine; one per agent.
fn manual_dir(agent: Agent) -> &'static str {
    match agent {
        Agent::Claude => "$HOME/.t3boat/signin-claude",
        Agent::Codex => "$HOME/.t3boat/signin-codex",
    }
}

/// What the person needs for a manual sign-in: the page, and for Codex the code to enter there.
/// `needs_code`: the page gives a code to send back with `sign_in_code`.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManualSignIn {
    pub url: String,
    pub user_code: Option<String>,
    pub needs_code: bool,
}

impl MachineRuntime {
    pub async fn sign_in(&self, agent: Agent) -> Result<()> {
        self.ensure_awake().await?;
        let id = self.sandbox_id();
        let boat = &self.boat();
        // The CLI hands the sign-in URL to $BROWSER; this one records it instead.
        let dir = boat
            .exec_ok(
                &id,
                &format!(
                    "d=$(mktemp -d); printf '#!/bin/sh\\necho \"$1\" > %s/url\\n' \"$d\" > \"$d/browser\"; \
                     chmod +x \"$d/browser\"; \
                     BROWSER=\"$d/browser\" nohup script -qfec '{}' \"$d/log\" >/dev/null 2>&1 & echo \"$d\"",
                    agent.login()
                ),
                30,
            )
            .await?;
        let dir = dir.lines().last().unwrap_or_default().trim().to_owned();
        // The machine must not fall asleep while the person is in the browser.
        let result = self.kept_awake(self.finish_sign_in(agent, &dir)).await;
        let _ = boat
            .exec(&id, &format!("rm -rf '{dir}'; {}", stop_login(agent.login())), 30)
            .await;
        result?;
        self.refresh_providers().await;
        Ok(())
    }

    /// T3 re-checks the provider and tells T3 Code; the sign-in nudge goes away with it, and the
    /// recorded config learns it too.
    async fn refresh_providers(&self) {
        let refreshed = upstream::call(
            &self.upstream_url(),
            &self.upstream_token(),
            "server.refreshProviders",
            serde_json::json!({}),
        )
        .await;
        if let Ok(result) = refreshed {
            self.cache.lock().unwrap().fold_providers(&result["providers"]);
            self.save_cache();
        }
    }

    /// The sign-in for t3boat-server, whose browser is on another computer: the login runs with
    /// no local callback. Claude shows a page that gives a code to paste back
    /// (`submit_sign_in_code`); Codex shows a one-time code to enter on its page and finds out by
    /// itself. Answers as soon as the page is known; the login finishes in the background, with
    /// the machine kept awake, and T3 is told when it does.
    pub async fn start_manual_sign_in(self: &std::sync::Arc<Self>, agent: Agent) -> Result<ManualSignIn> {
        self.ensure_awake().await?;
        let id = self.sandbox_id();
        let boat = self.boat();
        let dir = manual_dir(agent);
        // The FIFO is the login's stdin: a holder keeps it open, so the login waits for a code
        // instead of reading end-of-file, and `submit_sign_in_code` writes the code into it.
        boat.exec_ok(
            &id,
            &format!(
                "{stop} 2>/dev/null; rm -rf {dir}; mkdir -p {dir} && mkfifo {dir}/in && \
                 (nohup sleep {hold} > {dir}/in 2>/dev/null &) && \
                 (BROWSER=false nohup script -qfec '{login}' {dir}/log < {dir}/in >/dev/null 2>&1 &) && echo started",
                login = agent.manual_login(),
                stop = stop_login(agent.manual_login()),
                hold = LOGIN_TIMEOUT.as_secs(),
            ),
            30,
        )
        .await?;
        let started = Instant::now();
        let sign_in = loop {
            let log = boat.exec(&id, &format!("cat {dir}/log 2>/dev/null"), 30).await?.stdout;
            let found = match agent {
                Agent::Claude => claude_manual_url(&log).map(|url| ManualSignIn { url, user_code: None, needs_code: true }),
                Agent::Codex => codex_device(&log).map(|(url, code)| ManualSignIn { url, user_code: Some(code), needs_code: false }),
            };
            if let Some(found) = found {
                break found;
            }
            if started.elapsed() > URL_TIMEOUT {
                bail!("the {agent:?} login did not start on the machine");
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        };
        let this = self.clone();
        tokio::spawn(async move {
            let done = this.kept_awake(this.wait_signed_in(agent)).await;
            let _ = this
                .boat()
                .exec(&this.sandbox_id(), &format!("{}; rm -rf {dir}", stop_login(agent.manual_login())), 30)
                .await;
            match done {
                Ok(()) => this.refresh_providers().await,
                Err(e) => tracing::warn!("{agent:?} sign-in on {}: {e:#}", this.id()),
            }
        });
        Ok(sign_in)
    }

    /// The code Claude's sign-in page gave, into the login `start_manual_sign_in` left waiting.
    pub async fn submit_sign_in_code(&self, agent: Agent, code: &str) -> Result<()> {
        let code = code.trim();
        if code.is_empty() || code.len() > 512 || code.chars().any(|c| c.is_control()) {
            bail!("that is not a sign-in code");
        }
        let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, format!("{code}\n"));
        let dir = manual_dir(agent);
        self.boat()
            .exec_ok(
                &self.sandbox_id(),
                &format!("test -p {dir}/in || {{ echo 'no sign-in is waiting for a code' >&2; exit 1; }}; echo {b64} | base64 -d > {dir}/in"),
                30,
            )
            .await
            .map(drop)
    }

    async fn wait_signed_in(&self, agent: Agent) -> Result<()> {
        let started = Instant::now();
        loop {
            tokio::time::sleep(Duration::from_secs(3)).await;
            if self.boat().exec(&self.sandbox_id(), agent.status(), 30).await?.exit_code == Some(0) {
                return Ok(());
            }
            if started.elapsed() > LOGIN_TIMEOUT {
                bail!("the {agent:?} sign-in was not finished in the browser");
            }
        }
    }

    async fn finish_sign_in(&self, agent: Agent, dir: &str) -> Result<()> {
        let id = self.sandbox_id();
        let boat = &self.boat();
        let started = Instant::now();
        let url = loop {
            let out = boat.exec(&id, &format!("cat '{dir}/url' 2>/dev/null"), 30).await?;
            if let Some(url) = out.stdout.split_whitespace().find(|w| w.starts_with("https://")) {
                break url.to_owned();
            }
            if started.elapsed() > URL_TIMEOUT {
                bail!("the {agent:?} login did not start on the machine");
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        };
        let port = callback_port(&url).ok_or_else(|| anyhow!("the {agent:?} login has no local callback"))?;
        let _forward = boat
            .forward(&id, port)
            .await
            .with_context(|| format!("forwarding port {port} for the {agent:?} login"))?;
        crate::app::open_url(&url)?;
        loop {
            tokio::time::sleep(Duration::from_secs(3)).await;
            if boat.exec(&id, agent.status(), 30).await?.exit_code == Some(0) {
                return Ok(());
            }
            if started.elapsed() > LOGIN_TIMEOUT {
                bail!("the {agent:?} sign-in was not finished in the browser");
            }
        }
    }
}

/// The port of a `redirect_uri=http://localhost:<port>/...` (or 127.0.0.1) parameter.
fn callback_port(url: &str) -> Option<u16> {
    let url = url::Url::parse(url).ok()?;
    let redirect = url.query_pairs().find(|(k, _)| k == "redirect_uri")?.1;
    let redirect = url::Url::parse(&redirect).ok()?;
    matches!(redirect.host_str()?, "localhost" | "127.0.0.1").then_some(())?;
    redirect.port()
}

/// Claude's login with no local callback: the URL it prints for when no browser opened, whose
/// page shows a code to paste back into the login.
pub fn claude_manual_url(log: &str) -> Option<String> {
    let after = &log[log.find("visit:")? + "visit:".len()..];
    // The URL comes inside an OSC 8 hyperlink (`ESC ] 8 ; ; URL BEL|ESC\ text ESC ] 8 ; ; ...`),
    // or bare: either way the first https run, cut at any control character.
    let start = after.find("https://")?;
    let url: String = after[start..].chars().take_while(|c| !c.is_control() && !c.is_whitespace()).collect();
    url.contains("redirect_uri=").then_some(url)
}

/// Codex's `--device-auth` login: the page to open and the one-time code to enter there.
pub fn codex_device(log: &str) -> Option<(String, String)> {
    let plain = strip_ansi(log);
    let url = plain
        .split_whitespace()
        .find(|w| w.starts_with("https://") && w.contains("/device"))?
        .to_owned();
    let after = &plain[plain.find("one-time code")?..];
    let code = after.split_whitespace().find(|w| is_device_code(w))?.to_owned();
    Some((url, code))
}

/// `Q7VA-LK422`: letters and digits, a dash between two groups.
fn is_device_code(w: &str) -> bool {
    let Some((a, b)) = w.split_once('-') else { return false };
    let part = |p: &str, n: std::ops::RangeInclusive<usize>| {
        n.contains(&p.len()) && p.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
    };
    part(a, 4..=5) && part(b, 4..=6)
}

/// Drops ANSI CSI sequences (`ESC [ ... letter`), which colour the CLIs' output.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod manual_tests {
    use super::{claude_manual_url, codex_device};

    /// As `script` records `claude auth login` 2.1.326 with no browser: the URL twice inside an
    /// OSC 8 hyperlink, then the paste prompt.
    const CLAUDE: &str = "Opening browser to sign in\u{2026}\r\nIf the browser didn't open, visit: \u{1b}]8;;https://claude.com/cai/oauth/authorize?code=true&client_id=9d1c&response_type=code&redirect_uri=https%3A%2F%2Fplatform.claude.com%2Foauth%2Fcode%2Fcallback&state=abc\u{7}https://claude.com/cai/oauth/authorize?code=true&client_id=9d1c&response_type=code&redirect_uri=https%3A%2F%2Fplatform.claude.com%2Foauth%2Fcode%2Fcallback&state=abc\u{1b}]8;;\u{7}\r\nPaste code here if prompted > ";

    /// As `script` records `codex login --device-auth` 0.169.0.
    const CODEX: &str = "Welcome to Codex [v0.169.0]\r\n\u{1b}[1mFollow these steps to sign in with ChatGPT using device code authorization:\u{1b}[0m\r\n1. Open this link in your browser and sign in to your account\r\n   \u{1b}[94mhttps://auth.openai.com/codex/device\u{1b}[0m\r\n2. Enter this one-time code \u{1b}[90m(expires in 15 minutes)\u{1b}[0m\r\n   \u{1b}[94mQ7VA-LK422\u{1b}[0m\r\n";

    #[test]
    fn finds_claudes_paste_code_url() {
        let url = claude_manual_url(CLAUDE).unwrap();
        assert!(url.starts_with("https://claude.com/cai/oauth/authorize?"), "{url}");
        assert!(url.ends_with("state=abc"), "{url}");
        assert!(url.contains("redirect_uri=https%3A%2F%2Fplatform.claude.com"), "{url}");
        assert_eq!(claude_manual_url("Opening browser to sign in\u{2026}\r\n"), None);
    }

    #[test]
    fn finds_codexs_device_page_and_code() {
        assert_eq!(
            codex_device(CODEX),
            Some(("https://auth.openai.com/codex/device".to_owned(), "Q7VA-LK422".to_owned()))
        );
        assert_eq!(codex_device("Welcome to Codex [v0.169.0]\r\n"), None);
    }
}

#[cfg(test)]
mod tests {
    use super::callback_port;

    #[test]
    fn finds_the_local_callback_port() {
        let claude = "https://claude.com/cai/oauth/authorize?code=true&redirect_uri=http%3A%2F%2Flocalhost%3A43327%2Fcallback&state=x";
        let codex = "https://auth.openai.com/oauth/authorize?redirect_uri=http%3A%2F%2F127.0.0.1%3A1455%2Fauth%2Fcallback";
        let manual = "https://claude.com/cai/oauth/authorize?redirect_uri=https%3A%2F%2Fplatform.claude.com%2Foauth%2Fcode%2Fcallback";
        assert_eq!(callback_port(claude), Some(43327));
        assert_eq!(callback_port(codex), Some(1455));
        assert_eq!(callback_port(manual), None);
    }
}
