//! Signing an agent in on a machine with the vendor's own CLI login. The login runs on the
//! machine and its localhost callback port is forwarded to this computer, so the browser's
//! redirect after "Authorize" lands on the machine: no code to paste, and the credential is
//! created on the machine and stays there. This computer's own logins are never touched.

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

    /// Exits 0 once signed in.
    fn status(self) -> &'static str {
        match self {
            Self::Claude => "claude auth status >/dev/null 2>&1",
            Self::Codex => "codex login status >/dev/null 2>&1",
        }
    }
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
            .exec(&id, &format!("rm -rf '{dir}'; pkill -f '{}'", agent.login()), 30)
            .await;
        result?;
        // T3 re-checks the provider and tells T3 Code; the sign-in nudge goes away with it, and
        // the recorded config learns it too.
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
        Ok(())
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
pub fn claude_manual_url(_log: &str) -> Option<String> {
    None
}

/// Codex's `--device-auth` login: the page to open and the one-time code to enter there.
pub fn codex_device(_log: &str) -> Option<(String, String)> {
    None
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
