import { defineConfig } from "@playwright/test";

const testDist = process.env.FUNFERN_TEST_DIST ?? "dist";
const isolationArgument = process.env.FUNFERN_TEST_ISOLATED === "0" ? "" : " --isolated";
// Where the bundle lives on its host: the site is served under /funfern/,
// and a bundle built with that public URL is served under it here too.
const prefix = process.env.FUNFERN_TEST_PREFIX ?? "/";
const prefixArgument = prefix === "/" ? "" : ` --prefix ${JSON.stringify(prefix)}`;
const baseURL = `http://127.0.0.1:4173${prefix}`;
const webGpuArgs = ["--enable-unsafe-webgpu"];
if (process.platform === "linux") {
  webGpuArgs.push(
    "--enable-features=Vulkan",
    "--use-angle=vulkan",
    "--disable-vulkan-surface",
  );
}
// More flags for a run that needs them, space-separated: the software adapter's
// `--use-webgpu-adapter=swiftshader`.
webGpuArgs.push(...(process.env.FUNFERN_CHROMIUM_ARGS ?? "").split(" ").filter(Boolean));

export default defineConfig({
  testDir: "browser-tests",
  // The end-to-end fixtures need a bundle built with the e2e feature, and run
  // only from `npm run test:e2e`, which serves one.
  testIgnore: process.env.FUNFERN_E2E_BUNDLE ? [] : ["**/e2e.spec.mjs"],
  fullyParallel: false,
  workers: 1,
  timeout: 60_000,
  expect: { timeout: 15_000 },
  reporter: process.env.CI ? "line" : "list",
  use: {
    baseURL,
    browserName: "chromium",
    channel: process.env.PLAYWRIGHT_CHANNEL ?? "chromium",
    headless: true,
    launchOptions: { args: webGpuArgs },
    viewport: { width: 1100, height: 760 },
  },
  webServer: {
    command: `python3 browser-tests/server.py --port 4173 --directory ${JSON.stringify(testDist)}${isolationArgument}${prefixArgument}`,
    url: baseURL,
    reuseExistingServer: !process.env.CI,
    timeout: 15_000,
  },
});
