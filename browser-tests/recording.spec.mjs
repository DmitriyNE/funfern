import { expect, test } from "@playwright/test";
import { stat } from "node:fs/promises";

test("browser recorder captures the WebGPU canvas and downloads video", async ({ page }) => {
  await page.goto("/");
  const canvas = page.locator("canvas");
  await expect(canvas).toBeVisible();
  await expect
    .poll(async () =>
      canvas.evaluate((element) => element.width > 0 && element.height > 0),
    )
    .toBe(true);

  const findModule = () =>
    page.evaluate(async () => {
      const candidates = performance
        .getEntriesByType("resource")
        .map((entry) => entry.name)
        .filter((name) => name.includes("/snippets/") && name.endsWith(".js"));
      for (const candidate of candidates) {
        const module = await import(candidate);
        if (typeof module.funfernRecordingSupported === "function") return candidate;
      }
      return undefined;
    });
  await expect.poll(async () => (await findModule()) !== undefined).toBe(true);
  const moduleUrl = await findModule();

  const supported = await page.evaluate(async (url) => {
    const recorder = await import(url);
    return recorder.funfernRecordingSupported();
  }, moduleUrl);
  expect(supported).toBe(true);

  const downloadPromise = page.waitForEvent("download");
  await page.evaluate(async (url) => {
    const recorder = await import(url);
    recorder.funfernStartRecording(
      320,
      180,
      10,
      0,
      0,
      1,
      1,
      () => {},
      () => {},
      (error) => {
        throw new Error(error);
      },
    );
    await new Promise((resolve) => setTimeout(resolve, 700));
    recorder.funfernStopRecording();
  }, moduleUrl);
  const download = await downloadPromise;
  expect(download.suggestedFilename()).toMatch(/^funfern-recording\.(webm|mp4)$/);
  const path = await download.path();
  expect(path).not.toBeNull();
  expect((await stat(path)).size).toBeGreaterThan(0);
});
