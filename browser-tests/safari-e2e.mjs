// The app's end-to-end fixtures in real Safari, through safaridriver. No CI
// browser runs Safari's WebGPU, so this runs by hand on a Mac with "Allow
// remote automation" on in Safari's developer settings:
//
//   TRUNK_BUILD_FEATURES=browser-threads,e2e scripts/trunk build --release --dist dist-e2e
//   node browser-tests/safari-e2e.mjs            every fixture the device suite names
//   node browser-tests/safari-e2e.mjs cavity     the ones named
//
// It serves dist-e2e cross-origin isolated, opens `?e2e=<fixture>` in a
// WebDriver session, and waits for the verdict the driver writes on the page's
// root element, as browser-tests/e2e.spec.mjs does in Chrome. It speaks the
// WebDriver protocol directly, so it needs nothing installed. Exits 1 if any
// fixture failed.
import { spawn } from "node:child_process";
import { readFileSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";

const port = 4174;
const driverPort = 4445;
const deadline = Number(process.env.FUNFERN_E2E_DEADLINE ?? 120);
const named = process.argv.slice(2);
const fixtures = named.length
  ? named
  : [...readFileSync("scripts/device-suite.sh", "utf8").matchAll(/FUNFERN_E2E=([a-z-]+)/g)].map(
      (match) => match[1],
    );

const server = spawn(
  "python3",
  ["browser-tests/server.py", "--port", String(port), "--directory", "dist-e2e", "--isolated"],
  { stdio: "ignore" },
);
const driver = spawn("/usr/bin/safaridriver", ["-p", String(driverPort)], { stdio: "ignore" });

async function webdriver(method, path, body) {
  const response = await fetch(`http://127.0.0.1:${driverPort}${path}`, {
    method,
    headers: { "content-type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const reply = await response.json();
  if (!response.ok) {
    throw new Error(`${method} ${path}: ${JSON.stringify(reply.value)}`);
  }
  return reply.value;
}

async function until(ready, seconds, what) {
  const end = Date.now() + seconds * 1000;
  while (Date.now() < end) {
    try {
      const value = await ready();
      if (value) {
        return value;
      }
    } catch {
      // Not up yet.
    }
    await sleep(500);
  }
  throw new Error(`${what} within ${seconds} s`);
}

let session;
let failed = 0;
try {
  await until(() => fetch(`http://127.0.0.1:${port}/`).then((r) => r.ok), 15, "no server");
  await until(
    () => webdriver("GET", "/status").then((status) => status.ready),
    15,
    "no safaridriver",
  );
  session = (
    await webdriver("POST", "/session", {
      capabilities: { alwaysMatch: { browserName: "safari" } },
    })
  ).sessionId;
  const version = await webdriver("POST", `/session/${session}/execute/sync`, {
    script: "return navigator.userAgent;",
    args: [],
  });
  console.log(`Safari: ${version}`);
  for (const fixture of fixtures) {
    const started = Date.now();
    await webdriver("POST", `/session/${session}/url`, {
      url: `http://127.0.0.1:${port}/?e2e=${fixture}&e2e-deadline=${deadline}`,
    });
    const [verdict, summary] = await until(
      async () => {
        const state = await webdriver("POST", `/session/${session}/execute/sync`, {
          script:
            "const root = document.documentElement;" +
            "return [root.getAttribute('data-funfern-e2e'), root.getAttribute('data-funfern-e2e-summary')];",
          args: [],
        });
        return state[0] === "pass" || state[0] === "fail" ? state : null;
      },
      deadline + 60,
      `${fixture} gave no verdict`,
    ).catch((error) => ["fail", error.message]);
    const seconds = Math.round((Date.now() - started) / 1000);
    console.log(`${verdict === "pass" ? "pass" : "FAIL"} ${String(seconds).padStart(4)} s  ${fixture}: ${summary}`);
    if (verdict !== "pass") {
      failed += 1;
    }
  }
} finally {
  if (session) {
    await webdriver("DELETE", `/session/${session}`).catch(() => {});
  }
  driver.kill();
  server.kill();
}
console.log(`${fixtures.length - failed} of ${fixtures.length} passed in Safari`);
process.exit(failed === 0 ? 0 : 1);
