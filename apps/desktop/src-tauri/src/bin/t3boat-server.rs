//! t3boat without the tray: the machines, their proxies, the MCP server for coding agents, and
//! the UI as a web app (`web.rs`). For a host that is always on, such as a home or office
//! server, so machines keep waking on schedule with the laptop closed.
//!
//! Configured from the environment:
//!   T3BOAT_WEB_ADDR           where the web UI listens (default 127.0.0.1:47740)
//!   T3BOAT_WEB_ALLOWED_HOSTS  Host values it answers to, comma separated (default: the address's host)
//!   T3BOAT_WEB_DIST           the built UI (default: `share/t3boat/web` beside the binary)
//!   T3BOAT_DATA_DIR           machines, settings and spend (default ~/.local/share/t3boat)
//!   T3BOAT_MCP_ADDR           the MCP server for coding agents (default 127.0.0.1:47730)
//!   T3BOAT_SSH_HOST           this server's SSH name, for the forward pairing shows (optional)

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use t3boat_lib::app::{self, Account, App, McpStatus, Setup, UiState};
use t3boat_lib::machine::provision::SIZES;
use t3boat_lib::machine::signin::Agent;
use t3boat_lib::machine::Event;
use t3boat_lib::store::Settings;
use t3boat_lib::web::{self, Commands, WebConfig};
use tokio::sync::broadcast;

const COSTS_EVERY: Duration = Duration::from_secs(300);
const USAGE: &str = "t3boat-server: t3boat's machines and web UI, without the tray app.
Configured from the environment: T3BOAT_WEB_ADDR, T3BOAT_WEB_ALLOWED_HOSTS, T3BOAT_WEB_DIST,
T3BOAT_DATA_DIR, T3BOAT_MCP_ADDR, T3BOAT_SSH_HOST (see the README).";

struct Server {
    app: Mutex<Option<Arc<App>>>,
    setup: Mutex<Setup>,
    account: Mutex<Option<Account>>,
    data_dir: PathBuf,
    mcp_addr: SocketAddr,
    /// The name another computer reaches this server by over SSH, for pairing's port forward.
    ssh_host: Option<String>,
    changed: broadcast::Sender<()>,
    init_lock: tokio::sync::Mutex<()>,
}

impl Server {
    fn app(&self) -> Result<Arc<App>, String> {
        self.app.lock().unwrap().clone().ok_or_else(|| "t3boat is not set up yet".to_owned())
    }

    /// A pairing as the UI shows it: with the SSH name to forward the port from.
    fn with_ssh_host(&self, mut v: Value) -> Value {
        if let Some(o) = v.as_object_mut() {
            o.insert("sshHost".into(), json!(self.ssh_host));
        }
        v
    }

    fn changed(&self) {
        let _ = self.changed.send(());
    }

    /// Checks Boat and T3 and starts the app model once both are there. Again on "Check again".
    async fn init(self: &Arc<Self>) {
        let _init = self.init_lock.lock().await;
        if self.app.lock().unwrap().is_some() {
            return;
        }
        let (setup, boat, t3) = app::check_setup().await;
        *self.setup.lock().unwrap() = setup;
        if let Some(boat) = &boat {
            self.refresh_account(boat).await;
        }
        if let (Setup::Ready, Some(boat), Some(t3)) = (setup, boat, t3) {
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            match App::new(boat, t3, self.data_dir.clone(), tx) {
                Ok(app) => {
                    *self.app.lock().unwrap() = Some(app.clone());
                    tokio::spawn(self.clone().events(app.clone(), rx));
                    tokio::spawn(self.clone().background(app.clone()));
                    let addr = self.mcp_addr;
                    tokio::spawn(async move {
                        if let Err(e) = t3boat_lib::mcp::serve_on(app, addr).await {
                            tracing::error!("coding agents cannot reach t3boat: {e:#}");
                        }
                    });
                }
                Err(e) => tracing::error!("t3boat could not start: {e:#}"),
            }
        } else {
            tracing::warn!("t3boat is not set up: {setup:?}");
        }
        self.changed();
    }

    async fn refresh_account(&self, boat: &t3boat_lib::boat::Boat) {
        if let Ok(a) = boat.account().await {
            *self.account.lock().unwrap() = Some(Account { identifier: a.identifier, plan: a.plan });
        }
    }

    async fn events(self: Arc<Self>, app: Arc<App>, mut rx: tokio::sync::mpsc::UnboundedReceiver<Event>) {
        while let Some(ev) = rx.recv().await {
            match ev {
                Event::Persist => app.save(),
                Event::Changed => self.changed(),
                Event::Notify { title, body } => tracing::info!("{title}: {body}"),
            }
        }
    }

    async fn background(self: Arc<Self>, app: Arc<App>) {
        loop {
            app.refresh_costs().await;
            self.refresh_account(&app.shared.boat).await;
            self.changed();
            tokio::time::sleep(COSTS_EVERY).await;
        }
    }

    fn ui_state(&self) -> UiState {
        let app = self.app.lock().unwrap().clone();
        UiState {
            setup: *self.setup.lock().unwrap(),
            setup_busy: false,
            sign_in_code: None,
            account: self.account.lock().unwrap().clone(),
            machines: app.as_ref().map(|a| a.views()).unwrap_or_default(),
            costs: app.as_ref().map(|a| a.costs()).unwrap_or_default(),
            settings: app.as_ref().map(|a| a.settings()).unwrap_or_default(),
            sizes: SIZES,
            mcp: McpStatus::default(),
            host: "server",
        }
    }
}

/// A command's arguments, as the UI sends them (camelCase, like Tauri's).
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct Args {
    id: String,
    name: String,
    size: String,
    account: Option<String>,
    template: Option<String>,
    projects: Option<Vec<String>>,
    project_id: String,
    thread: Option<String>,
    agent: String,
    code: String,
    minutes: Option<u32>,
    discard: Option<bool>,
    settings: Option<Settings>,
    action: String,
}

fn err(e: anyhow::Error) -> String {
    format!("{e:#}")
}

fn agent(name: &str) -> Result<Agent, String> {
    Agent::from_name(name).ok_or_else(|| format!("{name} has to be signed in from T3 Code's settings"))
}

impl Commands for Server {
    async fn call(self: Arc<Self>, command: String, args: Value) -> Result<Value, String> {
        let a: Args = if args.is_null() {
            Args::default()
        } else {
            serde_json::from_value(args).map_err(|e| format!("bad arguments for {command}: {e}"))?
        };
        let done = |r: Result<(), String>| r.map(|()| Value::Null);
        match command.as_str() {
            "get_state" => Ok(self.state()),
            "create_machine" => {
                let account = a.account.filter(|x| !x.is_empty());
                let template = a.template.filter(|t| !t.trim().is_empty());
                let app = self.app()?;
                done(app
                    .create_machine(&a.name, &a.size, account.as_deref(), template.as_deref().map(str::trim), a.projects.unwrap_or_default())
                    .await
                    .map(drop)
                    .map_err(err))
            }
            "templates" => {
                let account = a.account.filter(|x| !x.is_empty());
                let t = self.app()?.templates(account.as_deref()).await.map_err(err)?;
                Ok(json!(t))
            }
            "local_projects" => Ok(json!(self.app()?.local_projects().await.map_err(err)?)),
            "add_project" => {
                let app = self.app()?;
                let machine = app.machine(&a.id).map_err(err)?;
                done(app.add_project(&machine, &a.project_id).await.map(drop).map_err(err))
            }
            "wake_machine" => done(self.app()?.machine(&a.id).map_err(err)?.ensure_awake().await.map_err(err)),
            "sleep_machine" => done(self.app()?.machine(&a.id).map_err(err)?.sleep_now().await.map_err(err)),
            // The browser is on another computer: it opens the link itself.
            "open_machine" => {
                let app = self.app()?;
                let machine = app.machine(&a.id).map_err(err)?;
                Ok(self.with_ssh_host(json!(app.t3_link(&machine, a.thread.as_deref()))))
            }
            "pair_machine" => {
                let app = self.app()?;
                let machine = app.machine(&a.id).map_err(err)?;
                Ok(self.with_ssh_host(json!(app.pairing(&machine))))
            }
            "latest_thread" => Ok(json!(self.app()?.machine(&a.id).map_err(err)?.latest_thread().await)),
            "get_machine_details" => Ok(json!(self.app()?.details(&a.id).map_err(err)?)),
            "sign_in_agent" => {
                let agent = agent(&a.agent)?;
                let machine = self.app()?.machine(&a.id).map_err(err)?;
                Ok(json!(machine.start_manual_sign_in(agent).await.map_err(err)?))
            }
            "sign_in_code" => {
                let agent = agent(&a.agent)?;
                let machine = self.app()?.machine(&a.id).map_err(err)?;
                done(machine.submit_sign_in_code(agent, &a.code).await.map_err(err))
            }
            "turn_on_connect" => done(self.app()?.machine(&a.id).map_err(err)?.turn_on_connect().await.map_err(err)),
            "keep_awake" => done(self.app()?.machine(&a.id).map_err(err)?.keep_awake(a.minutes).await.map_err(err)),
            "resize_machine" => done(self.app()?.machine(&a.id).map_err(err)?.resize(&a.size).await.map_err(err)),
            "delete_machine" => {
                let app = self.app()?;
                let machine = app.machine(&a.id).map_err(err)?;
                done(app.delete_machine(&machine, a.discard.unwrap_or(false)).await.map_err(err))
            }
            "update_settings" => {
                let settings = a.settings.ok_or("no settings given")?;
                app::check_settings(&settings).map_err(err)?;
                self.app()?.update_settings(settings).map_err(err)?;
                self.changed();
                Ok(Value::Null)
            }
            "setup_action" => match a.action.as_str() {
                "recheck" => {
                    self.init().await;
                    Ok(Value::Null)
                }
                _ => Err("On the server, install Boat and run `boat login` in a shell there, then check again.".into()),
            },
            "add_mcp" | "quit_app" => Err(format!("{command} is for the tray app, not the web UI")),
            other => Err(format!("unknown command {other}")),
        }
    }

    fn state(&self) -> Value {
        serde_json::to_value(self.ui_state()).unwrap_or(Value::Null)
    }
}

fn env_addr(name: &str, default: &str) -> Result<SocketAddr> {
    let v = std::env::var(name).unwrap_or_else(|_| default.to_owned());
    v.parse().with_context(|| format!("{name}={v} is not an address:port"))
}

/// The UI built into the package: `../share/t3boat/web` from the binary.
fn default_dist() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dist = exe.parent()?.parent()?.join("share/t3boat/web");
    dist.join("index.html").is_file().then_some(dist)
}

#[tokio::main]
async fn main() -> Result<()> {
    if std::env::args().skip(1).any(|a| a == "--help" || a == "-h") {
        println!("{USAGE}");
        return Ok(());
    }
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()))
        .init();
    let web_addr = env_addr("T3BOAT_WEB_ADDR", "127.0.0.1:47740")?;
    let mcp_addr = env_addr("T3BOAT_MCP_ADDR", "127.0.0.1:47730")?;
    let allowed_hosts: Vec<String> = match std::env::var("T3BOAT_WEB_ALLOWED_HOSTS") {
        Ok(v) => v.split(',').map(str::trim).filter(|h| !h.is_empty()).map(str::to_owned).collect(),
        Err(_) => vec![web_addr.ip().to_string()],
    };
    let dist = std::env::var_os("T3BOAT_WEB_DIST").map(PathBuf::from).or_else(default_dist);
    if dist.is_none() {
        tracing::warn!("no built UI (T3BOAT_WEB_DIST): serving the API only");
    }
    let data_dir = std::env::var_os("T3BOAT_DATA_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::data_dir().map(|d| d.join("t3boat")))
        .context("no data directory: set T3BOAT_DATA_DIR")?;

    let (changed, _) = broadcast::channel(64);
    let server = Arc::new(Server {
        app: Mutex::new(None),
        setup: Mutex::new(Setup::default()),
        account: Mutex::new(None),
        data_dir,
        mcp_addr,
        ssh_host: std::env::var("T3BOAT_SSH_HOST").ok().map(|h| h.trim().to_owned()).filter(|h| !h.is_empty()),
        changed: changed.clone(),
        init_lock: tokio::sync::Mutex::new(()),
    });
    server.init().await;

    let router = web::router(server.clone(), changed, WebConfig { allowed_hosts, dist });
    let listener = tokio::net::TcpListener::bind(web_addr).await.with_context(|| format!("listening on {web_addr}"))?;
    tracing::info!("t3boat web UI on http://{web_addr}");
    let saver = server.clone();
    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("SIGTERM handler");
            tokio::select! {
                _ = term.recv() => {}
                _ = tokio::signal::ctrl_c() => {}
            }
            if let Some(app) = saver.app.lock().unwrap().clone() {
                app.save();
            }
        })
        .await?;
    Ok(())
}
