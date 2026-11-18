import { readFileSync } from "node:fs";
import { expect, test } from "@playwright/test";
import { expectedBuildId } from "./build-id.mjs";

// The app's end-to-end fixtures (crates/funfern-app/src/ui/e2e.rs) in the
// browser: the bundle of the `funfern-fixtures` binary runs the fixture `?e2e=`
// names through the app's own handlers on the page's WebGPU device and writes its
// verdict on the root element. The list is the device suite's run lines, not
// its driver probe, so the native, software-Vulkan and browser runs cannot
// part. `npm run test:e2e` serves `dist-e2e`, built with
//   scripts/fixtures-page.sh && scripts/trunk build fixtures.html --release --dist dist-e2e
// RT-01's `interleaved-<seed>` runs are left out: their f64 reference
// follows the run on a thread of its own natively, and on the page would
// step inside its frames, setting the pace it is there to judge.
const allFixtures = [
  ...readFileSync("scripts/device-suite.sh", "utf8").matchAll(
    /'FUNFERN_E2E=([a-z0-9-]+) funfern-fixtures'/g,
  ),
]
  .map((match) => match[1])
  .filter((name) => !name.startsWith("interleaved-"));
// The ones to run, comma-separated, or all of them: CI's run on every push
// names `cavity`, which on SwiftShader takes minutes where Chrome takes seconds.
const named = (process.env.FUNFERN_E2E_FIXTURES ?? "")
  .split(",")
  .map((name) => name.trim())
  .filter(Boolean);
// And the ones to leave out of all of them, comma-separated: the nightly runs
// the recorders' fixtures and the late clocks', the newest handoffs and the
// handoff-memory fixtures as parts of their own, since alongside the rest
// they would carry the slower hosting past its cells' 340 minutes.
const excluded = (process.env.FUNFERN_E2E_EXCLUDE ?? "")
  .split(",")
  .map((name) => name.trim())
  .filter(Boolean);
const unknown = [...named, ...excluded].filter((name) => !allFixtures.includes(name));
if (unknown.length > 0) {
  throw new Error(`FUNFERN_E2E_FIXTURES or _EXCLUDE names no fixture the device suite runs: ${unknown}`);
}
const fixtures = (named.length > 0 ? named : allFixtures).filter(
  (name) => !excluded.includes(name),
);
const deadline = Number(process.env.FUNFERN_E2E_DEADLINE ?? 120);
// A fixture that runs longer than the rest takes the deadline in proportion:
// handoff-memory-running steps to 768 where the other handoff fixtures stop at
// 384, which take 19 to 25 minutes on SwiftShader against the full run's
// 1500 s, and it timed out there each night. The rest keep theirs, so a page
// that hangs is still caught at it.
// observables runs 1440 steps twice, its probes recording, where the cavity
// runs 256 once: 55 to 57 minutes a page on SwiftShader served as Pages, and
// past its 100 minutes isolated, beside the worker pool, on 2026-10-19 and
// 2026-10-20 both; far-field runs 1150 twice, and far-field-late once.
// observers runs the cavity's 256 twice, the second time recording, paused
// for a second and resized; superposition runs the walled cavity's 384 four
// times, as long as four of the handoff-memory pages, and similarity twice.
// baffle-move runs 512 steps of 33,938 nodes: 22.5 of its 25 minutes isolated
// on 2026-11-05, and past them served as Pages without the worker pool.
// batches runs a scene six times, and the nightly one scene a page
// (`e2e-scene=`, through FUNFERN_E2E_QUERY): Spatial soliton took 1,951 s on
// SwiftShader on an M1 Max on 2026-11-15 and the Josephson line 617 s, where
// the five together never finished in the runner's six hours; the double
// slit runs twice the steps; served without the pool its second page, cold,
// ran past 9,000 s in its last schedule where warm it took 1.8 hours
// (2026-11-18). lifecycle
// remeshes and adapts a scene twice, also a scene a page: on CI's SwiftShader
// the soliton's first cycle alone took 73 minutes, and served without the pool
// the self-sustained emitter's 80 (2026-11-17), its two past 9,000 s. preparation's replacing
// part asks two scenes for the Fine mesh four times each, 71 s on lavapipe
// with its workers; served without them, a page prepares and takes away a
// handoff's stationary flux a few milliseconds a frame; on 2026-11-16 CI's
// opened the double slit in 119 s, and an act may take a third of the run;
// on the 17th its Supersede ran past 1,500 s there, where the M1 Max's took
// 470 s with its page's CPU throttled four times.
// edges runs 46 cases,
// 25.6 minutes on the M1 Max's SwiftShader and about 100 s a case on CI's,
// where the page went silent at the 30th of its 50 minutes on 2026-11-16: a
// page of its own, the deadline its run's, 125 minutes. And the
// clock's three hand the walled cavity over eight times: CI's runner reached
// step 544 of clock-retimes' 640 at its 25 minutes, the M1 Max 10.
const runLength = {
  // Served without the pool, handoff-memory-remesh's warm start ran out of
  // its 1,500 s on CI where its cold start passed (2026-11-17).
  "handoff-memory-rebuild": 2,
  "handoff-memory-remesh": 2,
  "handoff-memory-edit": 2,
  "handoff-memory-timestep": 2,
  "handoff-memory-running": 2,
  "handoff-memory-running-late": 2,
  observables: 6,
  observers: 2,
  superposition: 4,
  similarity: 2,
  "far-field": 3,
  "far-field-late": 2,
  "baffle-move": 2,
  batches: 8,
  lifecycle: 10,
  preparation: 6,
  edges: 5,
  "clock-retimes": 2,
  "clock-retime-at-rebase": 2,
  "clock-retime-across-rebase": 2,
};
// How the bundle is hosted: `isolated`, the test server sending the headers that
// isolate the page; `pages`, as GitHub Pages serves the site - no headers, so the
// page installs coi-serviceworker.js and reloads itself once; or
// `pages-blocked`, the same with service workers refused, as a browser that
// blocks the site's storage refuses them, where the page runs without its
// worker pool after one reload. `npm run test:e2e:pages` and
// `test:e2e:pages-blocked` serve `dist-e2e-pages`, built with
//   scripts/trunk build fixtures.html --release --public-url /funfern/ --dist dist-e2e-pages
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
  expect(allFixtures.length).toBeGreaterThan(0);
});

// A run meant for one adapter must have had it, or it has not run what it is
// for: FUNFERN_EXPECT_ADAPTER names a word the adapter's description holds,
// `swiftshader` by default on SwiftShader. The app's own log cannot say - in
// the browser wgpu's adapter info is empty - so the fixture's page asks once
// its verdict is in; before it, the request would queue behind the shader
// compilation that holds SwiftShader's GPU process for minutes.
const expectedAdapter =
  process.env.FUNFERN_EXPECT_ADAPTER ??
  (process.env.FUNFERN_WEBGPU === "swiftshader" ? "swiftshader" : "");

// Seconds past a page's deadline its verdict may take: the driver reads its
// deadline between frames, and a frame of a batched schedule on CI's
// SwiftShader outlasted the minute this was until 2026-11-16, so a page past
// its time said nothing.
const grace = 900;
// Which starts a page on Pages takes: `cold`, `warm` or both, the default. The
// nightly runs the two as jobs of their own, since for its longest fixtures
// the two together outlast a job: lifecycle's ring resonator passed cold and
// ran out of its time warm (2026-11-16). A warm start alone takes the cold
// one's hosting path but not its run, leaving the cold page once its fixture
// has begun.
const starts = process.env.FUNFERN_E2E_START ?? "both";
if (!["cold", "warm", "both"].includes(starts)) {
  throw new Error(`FUNFERN_E2E_START is cold, warm or both, not ${starts}`);
}

for (const fixture of fixtures) {
  test(`${fixture} holds in the browser`, async ({ page }) => {
    const pageDeadline = deadline * (runLength[fixture] ?? 1);
    // A start on Pages runs the fixture twice, cold and warm.
    test.setTimeout((viaPages && starts === "both" ? 2 : 1) * (pageDeadline + grace + 90) * 1000);
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
      // A page that never gives its verdict still says how far it got: its
      // lines, which are otherwise printed with the verdict, before the
      // failure (the nightly's `batches` read as silent for 101 minutes).
      const outcome = await Promise.race([
        expect(root)
          .toHaveAttribute("data-funfern-e2e", /^(pass|fail)$/, {
            timeout: (pageDeadline + grace) * 1000,
          })
          .then(() => null),
        stopping,
      ]).catch((error) => {
        console.log(`${fixture} (${start}): no verdict in ${pageDeadline + grace} s`);
        for (const line of lines.splice(0)) {
          console.log(`  ${line}`);
        }
        throw error;
      });
      if (outcome !== null) {
        throw new Error(`the app stopped before its verdict: ${outcome}\n${lines.join("\n")}`);
      }
      const verdict = await root.getAttribute("data-funfern-e2e");
      const summary = await root.getAttribute("data-funfern-e2e-summary");
      console.log(`${fixture} (${start}): ${verdict}: ${summary}`);
      for (const line of lines.splice(0)) {
        console.log(`  ${line}`);
      }
      if (expectedAdapter) {
        const adapter = await page.evaluate(async () => {
          const info = (await navigator.gpu?.requestAdapter())?.info ?? {};
          return [info.vendor, info.architecture, info.device, info.description].join(" ");
        });
        console.log(`  adapter: ${adapter}`);
        expect(adapter.toLowerCase(), `${start}: the page's WebGPU adapter`).toContain(
          expectedAdapter.toLowerCase(),
        );
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

    await page.goto(`./?e2e=${fixture}&e2e-deadline=${pageDeadline}${query}`);
    // Navigations of the main frame on a cold start: the one asked for; on
    // Pages the reload once the service worker controls the page; refused,
    // the reload marked `isolation=off` and then the same-document one that
    // gives the address back without the marker. A warm start takes one.
    if (viaPages && starts === "warm") {
      await expect(page.locator("html")).toHaveAttribute("data-funfern-e2e", "running", {
        timeout: 600 * 1000,
      });
      await expect
        .poll(() => new URL(page.url()).searchParams.has("isolation"), { timeout: 60 * 1000 })
        .toBe(false);
    } else {
      await holds("cold", { isolated: 1, pages: 2, "pages-blocked": 3 }[hosting]);
    }
    if (viaPages && starts !== "cold") {
      navigations = 0;
      await page.reload();
      await holds("warm", 1);
    }
  });
}
