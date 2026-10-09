import { readFileSync } from "node:fs";
import { expect, test } from "@playwright/test";

// The app's end-to-end fixtures (crates/funfern-app/src/ui/e2e.rs) in the
// browser: a bundle built with the e2e feature runs the fixture `?e2e=` names
// through the app's own handlers on the page's WebGPU device and writes its
// verdict on the root element. The list is the device suite's, so the native,
// software-Vulkan and browser runs cannot part. `npm run test:e2e` serves
// `dist-e2e`, built with
//   TRUNK_BUILD_FEATURES=browser-threads,e2e scripts/trunk build --release --dist dist-e2e
const fixtures = [
  ...readFileSync("scripts/device-suite.sh", "utf8").matchAll(/FUNFERN_E2E=([a-z-]+)/g),
].map((match) => match[1]);
const deadline = Number(process.env.FUNFERN_E2E_DEADLINE ?? 120);
// Bevy's render error handler logs each of these before it stops rendering.
const fatalConsolePattern =
  /Caught rendering error|Caught DeviceLost error|Quitting the application due to \w+ RenderError|panicked at|RuntimeError: unreachable|WebGPU initialization failed/i;

test("the device suite names fixtures", () => {
  expect(fixtures.length).toBeGreaterThan(0);
});

for (const fixture of fixtures) {
  test(`${fixture} holds in the browser`, async ({ page }) => {
    test.setTimeout((deadline + 90) * 1000);
    const fatal = [];
    const lines = [];
    page.on("console", (message) => {
      const text = message.text();
      if (text.startsWith("e2e ")) {
        lines.push(text);
      }
      if (fatalConsolePattern.test(text)) {
        fatal.push(text);
      }
    });
    page.on("pageerror", (error) => fatal.push(error.message));

    await page.goto(`/?e2e=${fixture}&e2e-deadline=${deadline}`);
    const root = page.locator("html");
    await expect(root).toHaveAttribute("data-funfern-e2e", /^(pass|fail)$/, {
      timeout: (deadline + 60) * 1000,
    });
    const verdict = await root.getAttribute("data-funfern-e2e");
    const summary = await root.getAttribute("data-funfern-e2e-summary");
    console.log(`${fixture}: ${verdict}: ${summary}`);
    for (const line of lines) {
      console.log(`  ${line}`);
    }
    expect(fatal, fatal.join("\n\n")).toEqual([]);
    expect(verdict, `${summary}\n${lines.join("\n")}`).toBe("pass");
  });
}
