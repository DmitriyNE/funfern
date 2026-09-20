import { defineConfig } from "@playwright/test";

const testDist = process.env.FUNFERN_TEST_DIST ?? "dist";
const isolationArgument = process.env.FUNFERN_TEST_ISOLATED === "0" ? "" : " --isolated";
const webGpuArgs = ["--enable-unsafe-webgpu"];
if (process.platform === "linux") {
  webGpuArgs.push(
    "--enable-features=Vulkan",
    "--use-angle=vulkan",
    "--disable-vulkan-surface",
  );
}

export default defineConfig({
  testDir: "browser-tests",
  fullyParallel: false,
  workers: 1,
  timeout: 60_000,
  expect: { timeout: 15_000 },
  reporter: process.env.CI ? "line" : "list",
  use: {
    baseURL: "http://127.0.0.1:4173",
    browserName: "chromium",
    channel: process.env.PLAYWRIGHT_CHANNEL ?? "chromium",
    headless: true,
    launchOptions: { args: webGpuArgs },
    viewport: { width: 1100, height: 760 },
  },
  webServer: {
    command: `python3 browser-tests/server.py --port 4173 --directory ${JSON.stringify(testDist)}${isolationArgument}`,
    url: "http://127.0.0.1:4173/",
    reuseExistingServer: !process.env.CI,
    timeout: 15_000,
  },
});
