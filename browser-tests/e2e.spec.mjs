import { readFileSync } from "node:fs";
import { expect, test } from "@playwright/test";
import { expectedBuildId } from "./build-id.mjs";

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
// How the bundle is hosted: `isolated`, the test server sending the headers that
// isolate the page; `pages`, as GitHub Pages serves the site - no headers, so the
// page installs coi-serviceworker.js and reloads itself once; or
// `pages-blocked`, the same with service workers refused, as a browser that
// blocks the site's storage refuses them, where the page runs without its
// worker pool after one reload. `npm run test:e2e:pages` and
// `test:e2e:pages-blocked` serve `dist-e2e-pages`, built with
//   TRUNK_BUILD_FEATURES=browser-threads,e2e scripts/trunk build --release --public-url /funfern/ --dist dist-e2e-pages
// under that prefix.
const hosting = process.env.FUNFERN_E2E_HOSTING ?? "isolated";
const viaPages = hosting === "pages" || hosting === "pages-blocked";
// What the page ends up as: cross-origin isolated, controlled by a service
// worker, and the gate's one-shot reload flag in session storage.
const expectedPath = {
  isolated: [true, false, null],
  pages: [true, true, null],
  "pages-blocked": [false, false, "1"],
}[hosting];
test.use({ serviceWorkers: hosting === "pages-blocked" ? "block" : "allow" });
// The worker pool the page should end up with: the threaded bundle's, `active`;
// `unavailable`, the threaded bundle running its work on the main thread as it
// does where the page's shared-memory growth check fails (`?e2e-pool=off`
// makes it fail, through FUNFERN_E2E_QUERY); or `0`, the single-threaded
// bundle, which never writes the attribute.
const expectedWorker = process.env.FUNFERN_EXPECT_BROWSER_WORKER ?? "active";
// More of the page's query, `e2e-pool=off` for the main-thread run.
const query = (process.env.FUNFERN_E2E_QUERY ?? "")
  .split("&")
  .filter(Boolean)
  .map((parameter) => `&${parameter}`)
  .join("");
// Bevy's render error handler logs each of these before it stops rendering.
// The device-loss fixture loses the device on purpose, so for it the caught
// errors are expected; Bevy's quit is not, since the app's own handler
// replaces it, and a panic never is.
const fatalConsolePatternFor = (fixture) =>
  fixture === "device-loss"
    ? /Quitting the application due to \w+ RenderError|panicked at|RuntimeError: unreachable|WebGPU initialization failed/i
    : /Caught rendering error|Caught DeviceLost error|Quitting the application due to \w+ RenderError|panicked at|RuntimeError: unreachable|WebGPU initialization failed/i;

test("the device suite names fixtures", () => {
  expect(fixtures.length).toBeGreaterThan(0);
});

// A run meant for one adapter - SwiftShader in CI - must have it, or it has
// not run what it is for: FUNFERN_EXPECT_ADAPTER names a word its description
// holds.
const expectedAdapter = process.env.FUNFERN_EXPECT_ADAPTER;
if (expectedAdapter) {
  test(`the page's WebGPU adapter is ${expectedAdapter}`, async ({ page }) => {
    await page.goto("./");
    const info = await page.evaluate(async () => {
      const adapter = await navigator.gpu?.requestAdapter();
      const info = adapter?.info ?? {};
      return [info.vendor, info.architecture, info.device, info.description].join(" ");
    });
    console.log(`adapter: ${info}`);
    expect(info.toLowerCase()).toContain(expectedAdapter.toLowerCase());
  });
}

for (const fixture of fixtures) {
  test(`${fixture} holds in the browser`, async ({ page }) => {
    test.setTimeout((deadline + 90) * 1000);
    const fatal = [];
    const lines = [];
    // A lost device or a render error stops the app before the driver can
    // write a verdict: the first such message fails the fixture at once
    // rather than at its deadline.
    let stopped;
    const stopping = new Promise((resolve) => {
      stopped = resolve;
    });
    const fatalConsolePattern = fatalConsolePatternFor(fixture);
    page.on("console", (message) => {
      const text = message.text();
      if (text.startsWith("e2e ")) {
        lines.push(text);
      }
      if (fatalConsolePattern.test(text)) {
        fatal.push(text);
        stopped(text);
      }
    });
    page.on("pageerror", (error) => {
      fatal.push(error.message);
      stopped(error.message);
    });
    let navigations = 0;
    page.on("framenavigated", (frame) => {
      if (frame === page.mainFrame()) {
        navigations += 1;
      }
    });

    // Waits for the fixture's verdict and holds it, and the hosting path the
    // page took to run it: under the headers one navigation and no service
    // worker; on Pages the cold start registers the worker and reloads, two
    // navigations, and a warm start one, the worker already in place; both
    // leave the page isolated with its one-shot reload flag cleared.
    const holds = async (start, expectedNavigations) => {
      const root = page.locator("html");
      // The bundle served is the one built from this checkout.
      await expect(root).toHaveAttribute("data-funfern-build", expectedBuildId());
      const outcome = await Promise.race([
        expect(root)
          .toHaveAttribute("data-funfern-e2e", /^(pass|fail)$/, {
            timeout: (deadline + 60) * 1000,
          })
          .then(() => null),
        stopping,
      ]);
      if (outcome !== null) {
        throw new Error(`the app stopped before its verdict: ${outcome}\n${lines.join("\n")}`);
      }
      const verdict = await root.getAttribute("data-funfern-e2e");
      const summary = await root.getAttribute("data-funfern-e2e-summary");
      console.log(`${fixture} (${start}): ${verdict}: ${summary}`);
      for (const line of lines.splice(0)) {
        console.log(`  ${line}`);
      }
      const path = await page.evaluate(() => [
        crossOriginIsolated,
        Boolean(navigator.serviceWorker?.controller),
        sessionStorage.getItem("funfernCoiReload"),
        document.documentElement.getAttribute("data-funfern-preparation-worker"),
      ]);
      expect(navigations, `${start}: navigations of the main frame`).toBe(expectedNavigations);
      expect(path, `${start}: [isolated, worker controls the page, reload flag, pool]`).toEqual([
        ...expectedPath,
        expectedWorker === "0" ? null : expectedWorker,
      ]);
      // The address is the one asked for: the gate's marker never stays in it.
      expect(new URL(page.url()).searchParams.has("isolation"), `${start}: ${page.url()}`).toBe(
        false,
      );
      expect(fatal, fatal.join("\n\n")).toEqual([]);
      expect(verdict, `${summary}\n${lines.join("\n")}`).toBe("pass");
      if (fixture === "device-loss") {
        // The page was told: the stop on the root element and the overlay
        // back with the app's words.
        await expect(root).toHaveAttribute("data-funfern-stopped", /^(device-lost|render-error)$/);
        const overlay = await page.evaluate(() => [
          document.getElementById("startup").hidden,
          document.getElementById("startup-status").textContent,
        ]);
        expect(overlay[0]).toBe(false);
        expect(overlay[1]).toContain("funfern stopped");
        expect(overlay[1]).toContain("The document is saved");
      }
    };

    await page.goto(`./?e2e=${fixture}&e2e-deadline=${deadline}${query}`);
    // Navigations of the main frame on a cold start: the one asked for; on
    // Pages the reload once the service worker controls the page; refused,
    // the reload marked `isolation=off` and then the same-document one that
    // gives the address back without the marker. A warm start takes one.
    await holds("cold", { isolated: 1, pages: 2, "pages-blocked": 3 }[hosting]);
    if (viaPages) {
      navigations = 0;
      await page.reload();
      await holds("warm", 1);
    }
  });
}
