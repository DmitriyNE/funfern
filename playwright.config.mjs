import { defineConfig } from "@playwright/test";

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
    command: "python3 -m http.server 4173 --bind 127.0.0.1 --directory dist",
    url: "http://127.0.0.1:4173/",
    reuseExistingServer: !process.env.CI,
    timeout: 15_000,
  },
});
