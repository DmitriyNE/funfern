import { expect, test } from "@playwright/test";
import { readdir, readFile } from "node:fs/promises";

test("Chrome accepts every WGSL shader", async ({ page }) => {
  await page.route("**/", (route) =>
    route.fulfill({ contentType: "text/html", body: "<!doctype html><title>WGSL validation</title>" }),
  );
  await page.goto("/");

  const shaderDirectory = "crates/funfern-app/src";
  const shaderNames = (await readdir(shaderDirectory))
    .filter((name) => name.endsWith(".wgsl"))
    .sort();
  expect(shaderNames.length).toBeGreaterThanOrEqual(7);

  for (const name of shaderNames) {
    const code = await readFile(`${shaderDirectory}/${name}`, "utf8");
    const messages = await page.evaluate(async (source) => {
      const adapter = await navigator.gpu.requestAdapter();
      if (adapter === null) throw new Error("WebGPU adapter unavailable");
      const device = await adapter.requestDevice();
      const module = device.createShaderModule({ code: source });
      const info = await module.getCompilationInfo();
      device.destroy();
      return [...info.messages].map(({ message, type, lineNum, linePos }) => ({
        message,
        type,
        lineNum,
        linePos,
      }));
    }, code);
    const errors = messages.filter(({ type }) => type === "error");
    const diagnostics = messages
      .map(({ message, type, lineNum, linePos }) => `${type} ${lineNum}:${linePos}: ${message}`)
      .join("\n");
    expect(errors, `${name}\n${diagnostics}`).toEqual([]);
  }
});
