// Starts t3boat-server against the fakes (fake boat and t3 CLIs on PATH, fake machine on two
// ports) in throwaway directories, and stops it again.
import { spawn, type ChildProcess } from "node:child_process";
import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";

const here = path.dirname(new URL(import.meta.url).pathname);
const desktop = path.resolve(here, "..");
export const serverBin =
  process.env.T3BOAT_SERVER_BIN ?? path.join(desktop, "src-tauri/target/debug/t3boat-server");

async function freePort(): Promise<number> {
  return new Promise((resolve) => {
    const s = net.createServer().listen(0, "127.0.0.1", () => {
      const port = (s.address() as net.AddressInfo).port;
      s.close(() => resolve(port));
    });
  });
}

export interface Fixture {
  base: string;
  state: string;
  boatLog: () => string;
  machineLog: () => string;
  restart: () => Promise<void>;
  stop: () => Promise<void>;
}

async function waitFor(url: string, child: ChildProcess, ms = 30_000) {
  // A binary that cannot start reports it as an "error" event, not an exit.
  let failed: Error | null = null;
  child.once("error", (e) => (failed = e));
  const until = Date.now() + ms;
  while (Date.now() < until) {
    if (failed) throw failed;
    if (child.exitCode !== null) throw new Error(`t3boat-server exited with ${child.exitCode}`);
    try {
      const r = await fetch(url, { method: "POST" });
      if (r.ok) return;
    } catch {}
    await new Promise((r) => setTimeout(r, 200));
  }
  throw new Error(`t3boat-server did not answer at ${url}`);
}

export async function start(): Promise<Fixture> {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "t3boat-e2e-"));
  const state = path.join(root, "fake");
  const bin = path.join(root, "bin");
  const home = path.join(root, "home");
  for (const d of [state, bin, home]) fs.mkdirSync(d, { recursive: true });
  fs.symlinkSync(path.join(here, "fakes/boat.mjs"), path.join(bin, "boat"));
  fs.symlinkSync(path.join(here, "fakes/t3"), path.join(bin, "t3"));

  const [webPort, mcpPort, t3Port, watchdogPort] = await Promise.all([freePort(), freePort(), freePort(), freePort()]);
  const env = {
    ...process.env,
    HOME: home,
    PATH: `${bin}:${process.env.PATH}`,
    FAKE_STATE: state,
    FAKE_ENVIRONMENT_ID: "env-e2e",
    FAKE_T3_PORT: String(t3Port),
    FAKE_WATCHDOG_PORT: String(watchdogPort),
    FAKE_T3_URL: `http://127.0.0.1:${t3Port}`,
    FAKE_WATCHDOG_URL: `http://127.0.0.1:${watchdogPort}`,
    T3BOAT_DATA_DIR: path.join(root, "data"),
    T3BOAT_WEB_ADDR: `127.0.0.1:${webPort}`,
    T3BOAT_WEB_ALLOWED_HOSTS: "127.0.0.1",
    T3BOAT_WEB_DIST: path.join(desktop, "dist"),
    T3BOAT_MCP_ADDR: `127.0.0.1:${mcpPort}`,
    RUST_LOG: "info,t3boat_lib=debug",
  };
  const machine = spawn(process.execPath, [path.join(here, "fakes/machine.mjs")], { env, stdio: "inherit" });
  const serverLog = fs.openSync(path.join(root, "server.log"), "a");
  let server: ChildProcess;
  const launch = async () => {
    server = spawn(serverBin, [], { env, stdio: ["ignore", serverLog, serverLog] });
    await waitFor(`http://127.0.0.1:${webPort}/api/get_state`, server);
  };
  const halt = async () => {
    if (server.exitCode !== null) return;
    const exited = new Promise((r) => server.once("exit", r));
    server.kill("SIGTERM");
    await exited;
  };
  try {
    await launch();
  } catch (e) {
    // No fixture to stop later: take the fake machine and the temp dir down now.
    machine.kill("SIGTERM");
    fs.rmSync(root, { recursive: true, force: true });
    throw e;
  }
  const read =(name: string) => () => {
    const p = path.join(state, name);
    return fs.existsSync(p) ? fs.readFileSync(p, "utf8") : "";
  };
  return {
    base: `http://127.0.0.1:${webPort}`,
    state: root,
    boatLog: read("boat.log"),
    machineLog: read("machine.log"),
    restart: async () => {
      await halt();
      await launch();
    },
    stop: async () => {
      await halt();
      machine.kill("SIGTERM");
      // Keep the logs with the report; drop the rest.
      const keep = path.join(desktop, "e2e-results", "logs");
      fs.mkdirSync(keep, { recursive: true });
      for (const [from, to] of [
        [path.join(state, "boat.log"), "boat.log"],
        [path.join(state, "machine.log"), "machine.log"],
        [path.join(state, "t3.log"), "t3.log"],
        [path.join(root, "server.log"), "server.log"],
      ]) {
        if (fs.existsSync(from)) fs.copyFileSync(from, path.join(keep, to));
      }
      fs.rmSync(root, { recursive: true, force: true });
    },
  };
}
