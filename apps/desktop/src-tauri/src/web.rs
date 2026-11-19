//! The web UI's server: the same commands the tray app's UI calls, over HTTP, for t3boat running
//! headless (`t3boat-server`). `POST /api/<command>` takes the command's arguments as a JSON
//! object and answers its result as JSON, or a non-2xx with the error text as a JSON string.
//! `GET /api/events` streams a `state` event on connect and on every change. Everything else is
//! the built UI, with `index.html` for paths it does not have.
//!
//! No authentication: it is meant for a tailnet address. Requests must name an allowed host, and a
//! cross-site page cannot post to it: a POST whose `Origin` is not an allowed host is refused.

use axum::Router;
use serde_json::Value;
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::broadcast;

/// What the routes run; a trait so the HTTP layer is testable without a real app.
pub trait Commands: Send + Sync + 'static {
    /// Runs `command` with `args` (a JSON object; `null` for none).
    fn call(self: Arc<Self>, command: String, args: Value) -> impl Future<Output = Result<Value, String>> + Send;
    /// The whole UI state, as the `state` event carries it.
    fn state(&self) -> Value;
}

pub struct WebConfig {
    /// `Host` values the server answers to: a name, or a name with its port.
    pub allowed_hosts: Vec<String>,
    /// The built UI; none serves the API only.
    pub dist: Option<PathBuf>,
}

/// `changed` fires whenever the state may have changed.
pub fn router<C: Commands>(_commands: Arc<C>, _changed: broadcast::Sender<()>, _config: WebConfig) -> Router {
    Router::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// Answers `get_state` with the state, fails `fail`, echoes the rest.
    struct Fake {
        version: AtomicU32,
    }

    impl Commands for Fake {
        async fn call(self: Arc<Self>, command: String, args: Value) -> Result<Value, String> {
            match command.as_str() {
                "get_state" => Ok(self.state()),
                "fail" => Err("the machine is busy".into()),
                _ => Ok(json!({ "command": command, "args": args })),
            }
        }
        fn state(&self) -> Value {
            json!({ "version": self.version.load(Ordering::SeqCst) })
        }
    }

    struct Server {
        base: String,
        fake: Arc<Fake>,
        changed: broadcast::Sender<()>,
        _dist: Option<PathBuf>,
    }

    async fn serve(with_dist: bool) -> Server {
        let fake = Arc::new(Fake { version: AtomicU32::new(1) });
        let (changed, _) = broadcast::channel(16);
        let dist = with_dist.then(|| {
            let dir = std::env::temp_dir().join(format!("t3boat-web-test-{}", rand::random::<u64>()));
            std::fs::create_dir_all(dir.join("assets")).unwrap();
            std::fs::write(dir.join("index.html"), "<!doctype html><title>t3boat</title>").unwrap();
            std::fs::write(dir.join("assets/app.js"), "console.log(1)").unwrap();
            dir
        });
        let config = WebConfig {
            allowed_hosts: vec!["boat.example".into(), "127.0.0.1".into()],
            dist: dist.clone(),
        };
        let app = router(fake.clone(), changed.clone(), config);
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Server { base, fake, changed, _dist: dist }
    }

    fn client() -> reqwest::Client {
        reqwest::Client::new()
    }

    #[tokio::test]
    async fn a_command_answers_its_result_as_json() {
        let s = serve(false).await;
        let r = client()
            .post(format!("{}/api/wake_machine", s.base))
            .json(&json!({ "id": "m1" }))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let body: Value = r.json().await.unwrap();
        assert_eq!(body, json!({ "command": "wake_machine", "args": { "id": "m1" } }));
    }

    #[tokio::test]
    async fn a_command_without_a_body_gets_null_args() {
        let s = serve(false).await;
        let r = client().post(format!("{}/api/get_state", s.base)).send().await.unwrap();
        assert_eq!(r.status(), 200);
        assert_eq!(r.json::<Value>().await.unwrap(), json!({ "version": 1 }));
    }

    #[tokio::test]
    async fn a_failed_command_answers_its_error_text() {
        let s = serve(false).await;
        let r = client().post(format!("{}/api/fail", s.base)).json(&json!({})).send().await.unwrap();
        assert_eq!(r.status(), 400);
        assert_eq!(r.json::<Value>().await.unwrap(), json!("the machine is busy"));
    }

    #[tokio::test]
    async fn another_host_is_refused() {
        let s = serve(true).await;
        for (method, path) in [("POST", "/api/get_state"), ("GET", "/"), ("GET", "/api/events")] {
            let r = client()
                .request(method.parse().unwrap(), format!("{}{path}", s.base))
                .header("host", "evil.example")
                .send()
                .await
                .unwrap();
            assert_eq!(r.status(), 403, "{method} {path}");
        }
    }

    #[tokio::test]
    async fn an_allowed_host_may_carry_a_port() {
        let s = serve(false).await;
        let r = client()
            .post(format!("{}/api/get_state", s.base))
            .header("host", "boat.example:47740")
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
    }

    #[tokio::test]
    async fn a_post_from_another_origin_is_refused() {
        let s = serve(false).await;
        let post = |origin: &'static str| {
            client()
                .post(format!("{}/api/get_state", s.base))
                .header("host", "boat.example")
                .header("origin", origin)
                .send()
        };
        assert_eq!(post("https://evil.example").await.unwrap().status(), 403);
        assert_eq!(post("https://boat.example").await.unwrap().status(), 200);
        assert_eq!(post("http://boat.example:47740").await.unwrap().status(), 200);
    }

    #[tokio::test]
    async fn events_send_the_state_on_connect_and_on_change() {
        let s = serve(false).await;
        let mut r = client().get(format!("{}/api/events", s.base)).send().await.unwrap();
        assert_eq!(r.status(), 200);
        assert!(r.headers()["content-type"].to_str().unwrap().starts_with("text/event-stream"));
        let mut seen = String::new();
        read_until(&mut r, &mut seen, "\"version\":1").await;
        assert!(seen.contains("event: state"), "{seen}");
        s.fake.version.store(2, Ordering::SeqCst);
        s.changed.send(()).unwrap();
        read_until(&mut r, &mut seen, "\"version\":2").await;
    }

    async fn read_until(r: &mut reqwest::Response, seen: &mut String, needle: &str) {
        while !seen.contains(needle) {
            let chunk = tokio::time::timeout(std::time::Duration::from_secs(5), r.chunk())
                .await
                .expect("an event in time")
                .unwrap()
                .expect("the stream stays open");
            seen.push_str(&String::from_utf8_lossy(&chunk));
        }
    }

    #[tokio::test]
    async fn the_ui_is_served_with_index_as_fallback() {
        let s = serve(true).await;
        let get = |path: &'static str| client().get(format!("{}{path}", s.base)).send();
        let index = get("/").await.unwrap();
        assert_eq!(index.status(), 200);
        assert!(index.text().await.unwrap().contains("<title>t3boat</title>"));
        let asset = get("/assets/app.js").await.unwrap();
        assert_eq!(asset.status(), 200);
        assert_eq!(asset.text().await.unwrap(), "console.log(1)");
        let deep = get("/machines/m1").await.unwrap();
        assert_eq!(deep.status(), 200);
        assert!(deep.text().await.unwrap().contains("<title>t3boat</title>"));
        assert_eq!(get("/api/nope/extra").await.unwrap().status(), 404);
    }
}
