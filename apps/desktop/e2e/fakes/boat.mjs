#!/usr/bin/env node
// Fake Boat CLI for the web E2E. Keeps sandboxes in $FAKE_STATE/boat.json, logs every call to
// $FAKE_STATE/boat.log, and points `host` at the fake machine (fakes/machine.mjs).
import fs from "node:fs";
import path from "node:path";

const dir = process.env.FAKE_STATE;
const statePath = path.join(dir, "boat.json");
const load = () => (fs.existsSync(statePath) ? JSON.parse(fs.readFileSync(statePath, "utf8")) : { next: 1, sandboxes: {} });
const save = (s) => fs.writeFileSync(statePath, JSON.stringify(s, null, 2));
let args = process.argv.slice(2);
if (args[0] === "--json") args = args.slice(1);
fs.appendFileSync(path.join(dir, "boat.log"), args.join(" ").replace(/\n/g, "\\n").slice(0, 300) + "\n");
const out = (v) => console.log(JSON.stringify(v));
const fail = (msg) => {
  console.error(msg);
  process.exit(1);
};
const s = load();
const sandbox = (id) => s.sandboxes[id] ?? fail(`sandbox ${id} not found`);
const view = (sb) => ({ id: sb.id, name: sb.name, state: sb.state, stoppedAt: sb.state === "stopped" ? sb.stoppedAt : null });

switch (args[0]) {
  case "status":
    out({ account: { identifier: "e2e@example.invalid", signedIn: true, plan: "e2e" } });
    break;
  case "new": {
    const id = `bx_e2e${s.next++}`;
    s.sandboxes[id] = { id, name: args[args.indexOf("--name") + 1], state: "running", stoppedAt: null };
    save(s);
    out({ id });
    break;
  }
  case "info":
    out({ sandbox: view(sandbox(args[1])) });
    break;
  case "list":
    out({ sandboxes: Object.values(s.sandboxes).map(view) });
    break;
  case "resume": {
    const sb = sandbox(args[1]);
    sb.state = "running";
    sb.stoppedAt = null;
    save(s);
    out({ ok: true });
    break;
  }
  case "stop": {
    const sb = sandbox(args[1]);
    sb.state = "stopped";
    sb.stoppedAt = new Date().toISOString();
    save(s);
    out({ ok: true });
    break;
  }
  case "delete":
    sandbox(args[1]);
    delete s.sandboxes[args[1]];
    save(s);
    out({ ok: true });
    break;
  case "scp":
    out({ ok: true });
    break;
  case "host": {
    const port = Number(args[2]);
    // provision hosts T3 (3773) first and the watchdog second; any other port is the watchdog.
    out({ url: port === 3773 ? process.env.FAKE_T3_URL : process.env.FAKE_WATCHDOG_URL });
    break;
  }
  case "exec": {
    const id = args[1];
    sandbox(id);
    const script = args[args.length - 1];
    if (/^bash \/tmp\/t3boat-provision-/.test(script)) {
      const result = JSON.stringify({ environmentId: process.env.FAKE_ENVIRONMENT_ID, upstreamToken: "e2e-upstream" });
      out({ exitCode: 0, stdout: `${result}\n`, stderr: "" });
    } else {
      out({ exitCode: 0, stdout: "", stderr: "" });
    }
    break;
  }
  case "snapshots":
    out({ named: [] });
    break;
  case "snapshot":
    out({ ok: true });
    break;
  case "usage":
    out({ dollars: 0, seconds: 0 });
    break;
  case "limits":
    out({ starts: { day: { used: 0, limit: 200 } } });
    break;
  default:
    fail(`fake boat: unsupported ${args.join(" ")}`);
}
