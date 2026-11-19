#!/usr/bin/env node
// The fake machine for the web E2E: the T3 server t3boat talks to upstream, and the watchdog.
// A drain stops the sandbox in the fake Boat's state, as the real watchdog's `boat stop self` would.
//   node machine.mjs   (FAKE_STATE, FAKE_ENVIRONMENT_ID, FAKE_T3_PORT, FAKE_WATCHDOG_PORT)
import fs from "node:fs";
import http from "node:http";
import path from "node:path";

const dir = process.env.FAKE_STATE;
const environmentId = process.env.FAKE_ENVIRONMENT_ID;
const log = (line) => fs.appendFileSync(path.join(dir, "machine.log"), `${line}\n`);

function json(res, status, body) {
  res.writeHead(status, { "content-type": "application/json" });
  res.end(JSON.stringify(body));
}

function stopSandboxes() {
  const statePath = path.join(dir, "boat.json");
  if (!fs.existsSync(statePath)) return;
  const s = JSON.parse(fs.readFileSync(statePath, "utf8"));
  for (const sb of Object.values(s.sandboxes)) {
    sb.state = "stopped";
    sb.stoppedAt = new Date().toISOString();
  }
  fs.writeFileSync(statePath, JSON.stringify(s, null, 2));
}

let phase = "active";
const status = () => ({
  phase,
  busy: false,
  workingThreads: 0,
  workingSubagents: 0,
  cpuBusy: false,
  agentsUnknown: false,
  keepAwakeSecs: 0,
  keepAwakeForever: false,
  idleSecs: 10,
  sleepAfterSecs: 300,
  nextDueAt: null,
  workDue: false,
  manifest: null,
});

const t3 = http.createServer((req, res) => {
  log(`t3 ${req.method} ${req.url}`);
  if (req.url.startsWith("/.well-known/t3/environment")) {
    return json(res, 200, { environmentId, label: "e2e", capabilities: { mcpModeLimitHeader: true }, linkFence: 1 });
  }
  if (req.url.startsWith("/api/auth/session")) return json(res, 200, { authenticated: true });
  json(res, 404, { error: "fake T3 has no such route" });
});

const watchdog = http.createServer((req, res) => {
  let body = "";
  req.on("data", (c) => (body += c));
  req.on("end", () => {
    log(`watchdog ${req.method} ${req.url} ${body}`);
    if (req.url.startsWith("/drain")) {
      phase = "stopping";
      const reply = status();
      stopSandboxes();
      phase = "active";
      return json(res, 200, reply);
    }
    json(res, 200, status());
  });
});

t3.listen(Number(process.env.FAKE_T3_PORT), "127.0.0.1");
watchdog.listen(Number(process.env.FAKE_WATCHDOG_PORT), "127.0.0.1");
