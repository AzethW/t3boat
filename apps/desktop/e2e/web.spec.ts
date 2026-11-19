// Web UI end to end: t3boat-server with fake Boat and T3 behind it, driven in Chromium.
//   bun run build && cargo build --no-default-features --bin t3boat-server   (once)
//   bunx playwright test
import { expect, test } from "@playwright/test";
import { start, type Fixture } from "./server";

let fx: Fixture;
test.beforeAll(async () => {
  fx = await start();
});
test.afterAll(async () => {
  await fx?.stop();
});

test("the page loads its state over /api/events", async ({ page }) => {
  const events = page.waitForResponse((r) => r.url().endsWith("/api/events"));
  await page.goto(fx.base);
  expect((await events).status()).toBe(200);
  await expect(page.getByText("No machines yet")).toBeVisible();
});

test("create, sleep, wake and delete a machine", async ({ page }) => {
  await page.goto(fx.base);
  await page.getByRole("button", { name: "New machine" }).first().click();
  await page.locator("#machine-name").fill("alpha");
  await page.getByRole("button", { name: "Launch machine" }).click();

  const row = page.getByRole("button", { name: "Details for alpha" });
  await expect(row).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => fx.boatLog()).toMatch(/^new --name t3boat · alpha /m);

  const menu = (item: string) => async () => {
    await page.getByRole("button", { name: "More for alpha" }).click();
    await page.getByRole("menuitem", { name: item }).click();
  };
  // Sleep and wake are in the row's menu: an unpaired machine's button is "Add to T3 Code".
  await expect(page.getByText(/Idle/)).toBeVisible({ timeout: 30_000 });
  await menu("Sleep now")();
  await expect.poll(() => fx.machineLog(), { timeout: 15_000 }).toMatch(/watchdog POST \/drain/);
  await expect(page.getByText(/Asleep/)).toBeVisible({ timeout: 30_000 });

  await menu("Wake now")();
  await expect.poll(() => fx.boatLog(), { timeout: 15_000 }).toMatch(/^resume bx_e2e1 /m);
  await expect(page.getByText(/Idle/)).toBeVisible({ timeout: 30_000 });

  await page.getByRole("button", { name: "More for alpha" }).click();
  await page.getByRole("menuitem", { name: "Delete machine…" }).click();
  await page.getByRole("menuitem", { name: "Click again to delete forever" }).click();
  await expect(row).toHaveCount(0, { timeout: 30_000 });
  await expect.poll(() => fx.boatLog()).toMatch(/^delete bx_e2e1 --yes/m);
});

test("another Host is refused", async () => {
  for (const [method, path] of [["POST", "/api/get_state"], ["GET", "/"], ["GET", "/api/events"]]) {
    // fetch refuses to set Host, so speak HTTP directly.
    const status = await rawStatus(fx.base, method, path, "evil.example");
    expect(status, `${method} ${path}`).toBe(403);
  }
});

async function rawStatus(base: string, method: string, path: string, host: string): Promise<number> {
  const { port } = new URL(base);
  const net = await import("node:net");
  return new Promise((resolve, reject) => {
    const sock = net.connect(Number(port), "127.0.0.1", () => {
      sock.write(`${method} ${path} HTTP/1.1\r\nHost: ${host}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n`);
    });
    let data = "";
    sock.on("data", (d) => {
      data += d;
      const m = data.match(/^HTTP\/1\.1 (\d{3})/);
      if (m) {
        resolve(Number(m[1]));
        sock.destroy();
      }
    });
    sock.on("error", reject);
  });
}

/** WCAG contrast of an element's text against its own background. */
async function textContrast(page: import("@playwright/test").Page, selector: string): Promise<number> {
  return page.locator(selector).evaluate((el) => {
    const rgb = (c: string) => (c.match(/[\d.]+/g) ?? []).slice(0, 3).map(Number);
    const lum = ([r, g, b]: number[]) => {
      const f = (v: number) => ((v /= 255) <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4);
      return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
    };
    const s = getComputedStyle(el);
    const [a, b] = [lum(rgb(s.color)), lum(rgb(s.backgroundColor))].sort((x, y) => y - x);
    return (a + 0.05) / (b + 0.05);
  });
}

for (const scheme of ["dark", "light"] as const) {
  test(`pairing names the server and reads in ${scheme} mode`, async ({ page }) => {
    await page.emulateMedia({ colorScheme: scheme });
    await fetch(`${fx.base}/api/create_machine`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ name: `pair-${scheme}`, size: "default" }),
    });
    await page.goto(fx.base);
    await page.getByRole("button", { name: "Add to T3 Code" }).first().click();
    const command = page.getByRole("dialog").locator(".copyable");
    await command.waitFor();
    await expect.soft(command).toHaveText(/^ssh -N -L (\d+):localhost:\1 e2e-server$/);
    expect.soft(await textContrast(page, ".server-dialog .copyable"), "command text contrast").toBeGreaterThanOrEqual(4.5);
  });
}

test("the server stops promptly while a page holds its event stream open", async ({ page }) => {
  await page.goto(fx.base);
  await expect(page.getByText(/machines|No machines yet/).first()).toBeVisible();
  // systemd allows 90 s before SIGKILL; a stop that waits on the open stream never ends by itself.
  const started = Date.now();
  await fx.restart();
  expect(Date.now() - started, "restart time (ms)").toBeLessThan(10_000);
});
