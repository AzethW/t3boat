import { defineConfig, devices } from "@playwright/test";

// One server with fakes per run, so the specs run in order in one worker.
export default defineConfig({
  testDir: "e2e",
  workers: 1,
  fullyParallel: false,
  timeout: 120_000,
  outputDir: "e2e-results/artifacts",
  reporter: [
    ["list"],
    ["junit", { outputFile: "e2e-results/junit.xml" }],
    ["html", { outputFolder: "e2e-results/html", open: "never" }],
  ],
  use: {
    ...devices["Desktop Chrome"],
    viewport: { width: 1920, height: 1080 },
    trace: "on",
    screenshot: "on",
  },
});
