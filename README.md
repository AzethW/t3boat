<a href="https://t3boat.paukraft.com">
  <img src="apps/web/public/og.png" alt="t3boat: Run your T3 Code agents in the cloud, on Boat">
</a>

<p align="center">
  <a href="https://t3boat.paukraft.com">Website</a> ·
  <a href="#install">Install</a> ·
  <a href="apps/desktop/docs/architecture.md">How it works</a>
</p>

t3boat creates cloud machines on your [Boat](https://boat.dev) account and adds them to [T3 Code](https://t3.codes). They sleep when idle, so you only pay for the minutes your agents run.

Chat in T3 Code as usual. A message to a sleeping machine wakes it; running work finishes with your laptop closed.

## What you get

- Create, resize and delete machines from the menu bar, plain or from a saved Boat snapshot with your tools on it
- Add projects the way T3 Code's "Add to another machine" does, `.env` files included
- Sign Claude Code or Codex in on a machine with their own login
- T3 Connect, to reach a machine from your phone or browser
- Cost per machine, today and this month
- Machines in another Boat account or organization, its API key read from the macOS keychain
- An MCP server, so agents on your computer can manage machines

## What you need

- [T3 Code](https://t3.codes) desktop app
- A [Boat](https://boat.dev) account (t3boat walks you through the CLI sign-in)
- macOS, Windows or Linux

## Install

No prebuilt download yet. Build from source with [Rust](https://rustup.rs) and [Bun](https://bun.sh). Windows also needs the [Microsoft C++ Build Tools](https://v2.tauri.app/start/prerequisites/#windows), Linux [Tauri's system packages](https://v2.tauri.app/start/prerequisites/#linux) (Debian/Ubuntu: `sudo apt install libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev libssl-dev build-essential file`).

```sh
git clone https://github.com/paukraft/t3boat && cd t3boat
bun install
bun run build:desktop
```

The app lands in `apps/desktop/src-tauri/target/release/bundle/`:

- **macOS**: copy `macos/t3boat.app` to `/Applications` and open it. It lives in the menu bar.
- **Windows**: run `nsis/t3boat_*-setup.exe`. It lives in the system tray, maybe behind the overflow arrow.
- **Linux**: install `deb/t3boat_*.deb` or run the AppImage in `appimage/`. It lives in the system tray; without one (GNOME without AppIndicator) it opens as a window.

Or let your agent do it:

```
Please build and install t3boat (https://github.com/paukraft/t3boat) on my computer: clone it, follow the Install section of its README, and open the app when it's done.
```

## Usage

1. Click the boat in the menu bar and create a machine.
2. Accept the new backend in T3 Code.
3. Work in T3 Code as usual. Push or open a PR from the machine.

To let Claude Code or Codex on your computer manage machines, click "Add" next to them in t3boat's settings.

## Headless server

`t3boat-server` is t3boat without the menu bar app, for a computer that is always on: a home or office server running T3 Code's headless server. Machines keep waking on schedule with your laptop closed, and the UI is a web page.

```sh
nix build github:AzethW/t3boat/jmb/headless-web#t3boat-server   # binary plus the built UI
```

Without Nix: `bun install`, `bun run --cwd apps/desktop build`, then `cargo build --release --no-default-features --bin t3boat-server` in `apps/desktop/src-tauri`, and point `T3BOAT_WEB_DIST` at `apps/desktop/dist`.

The server needs the Boat CLI signed in (`boat login`) and `t3` on `PATH` or in `~/.t3/bin`. It is configured from the environment:

| Variable | Default | |
|---|---|---|
| `T3BOAT_WEB_ADDR` | `127.0.0.1:47740` | where the web UI listens |
| `T3BOAT_WEB_ALLOWED_HOSTS` | the address's host | `Host` values it answers to, comma separated; anything else gets 403 |
| `T3BOAT_WEB_DIST` | `share/t3boat/web` beside the binary | the built UI |
| `T3BOAT_DATA_DIR` | `~/.local/share/t3boat` | machines, settings and spend |
| `T3BOAT_MCP_ADDR` | `127.0.0.1:47730` | the MCP server for coding agents on the server |
| `T3BOAT_SSH_HOST` | none | the server's SSH name, shown in the port forward that pairing needs |

The web UI has no login. Bind it to a private address, such as a tailnet one, and list the names you reach it by in `T3BOAT_WEB_ALLOWED_HOSTS`. POSTs from another site's page are refused.

What differs from the menu bar app:
- **Adding a machine to T3 Code** on another computer needs its proxy port: the UI shows the `ssh -L` forward to run there, then the `t3code://` link.
- **Signing an agent in** happens on the vendor's page in your browser: Claude's page gives a code to paste back into t3boat; Codex shows a one-time code to enter on its page.
- **T3 Connect's sign-in** opens a browser on the computer t3boat runs on, so turn it on from the menu bar app.

## Development

```sh
bun run dev           # website (apps/web)
bun run dev:desktop   # menu bar app (apps/desktop, Tauri)
nix develop           # Rust, Bun, Tauri libraries and Playwright browsers
```

Web E2E: `t3boat-server` with fake `boat` and `t3` CLIs and a fake machine, driven in Chromium. It writes a JUnit report, traces, screenshots and the fakes' logs to `apps/desktop/e2e-results/`.

```sh
cd apps/desktop
bun run build && bun run build:server && bunx playwright test
```

```sh
bun run dev           # website (apps/web)
bun run dev:desktop   # menu bar app (apps/desktop, Tauri)
```

Architecture: [apps/desktop/docs/architecture.md](apps/desktop/docs/architecture.md).

## License

[MIT](LICENSE). Made by [Pau Kraft](https://x.com/paukraft). Not affiliated with or endorsed by T3 Code or Boat.
