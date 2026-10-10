import { defineConfig } from "@playwright/test";

const testDist = process.env.FUNFERN_TEST_DIST ?? "dist";
const isolationArgument = process.env.FUNFERN_TEST_ISOLATED === "0" ? "" : " --isolated";
// Where the bundle lives on its host: the site is served under /funfern/,
// and a bundle built with that public URL is served under it here too.
const prefix = process.env.FUNFERN_TEST_PREFIX ?? "/";
const prefixArgument = prefix === "/" ? "" : ` --prefix ${JSON.stringify(prefix)}`;
const baseURL = `http://127.0.0.1:4173${prefix}`;
// The WebGPU the browser runs on: the machine's GPU, or `FUNFERN_WEBGPU=swiftshader`
// on a Linux machine without one, as CI's runner is - Chromium's bundled
// software Vulkan, SwiftShader, as the page's adapter and as the compositor's
// Vulkan too. With the compositor on anything else the canvas's swap chain finds
// no shared image both sides can use, Chromium drops the page's WebGPU, and the
// app's device is lost as it starts ("A valid external Instance reference no
// longer exists"). Chromium offers no other software adapter: Dawn finds Mesa's
// lavapipe, but Chromium blocklists every CPU adapter save SwiftShader.
const webGpu = process.env.FUNFERN_WEBGPU ?? "native";
if (!["native", "swiftshader"].includes(webGpu)) {
  throw new Error(`FUNFERN_WEBGPU is native or swiftshader, not ${webGpu}`);
}
const webGpuArgs = ["--enable-unsafe-webgpu"];
if (webGpu === "swiftshader") {
  webGpuArgs.push(
    "--use-webgpu-adapter=swiftshader",
    "--enable-unsafe-swiftshader",
    "--enable-features=Vulkan",
    "--use-vulkan=swiftshader",
    "--use-angle=swiftshader",
  );
} else if (process.platform === "linux") {
  webGpuArgs.push(
    "--enable-features=Vulkan",
    "--use-angle=vulkan",
    "--disable-vulkan-surface",
  );
}
// More flags for a run that needs them, space-separated.
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
