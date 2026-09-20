import { expect, test } from "@playwright/test";

const fatalConsolePattern =
  /Caught rendering error|Quitting the application due to Validation RenderError|panicked at|RuntimeError: unreachable|WebGPU initialization failed/i;
const expectPreparationWorker = process.env.FUNFERN_EXPECT_BROWSER_WORKER !== "0";

test("WebGPU app starts, advances, and resizes its render target", async ({ page }) => {
  const fatalMessages = [];
  const consoleMessages = [];

  page.on("console", (message) => {
    const text = message.text();
    consoleMessages.push(text);
    if (fatalConsolePattern.test(text)) {
      fatalMessages.push(text);
    }
  });
  page.on("pageerror", (error) => fatalMessages.push(error.message));

  await page.goto("/");
  expect(await page.evaluate(() => crossOriginIsolated)).toBe(expectPreparationWorker);
  if (expectPreparationWorker) {
    await expect
      .poll(() =>
        page.evaluate(() =>
          document.documentElement.getAttribute("data-funfern-preparation-worker"),
        ),
      )
      .toBe("active");
  } else {
    expect(
      await page.evaluate(() =>
        document.documentElement.getAttribute("data-funfern-preparation-worker"),
      ),
    ).toBeNull();
  }
  const canvas = page.locator("canvas");
  await expect(canvas).toBeVisible();
  await expect
    .poll(() => consoleMessages.some((message) => message.includes("BrowserWebGpu")))
    .toBe(true);

  const adapterAvailable = await page.evaluate(async () => {
    if (!navigator.gpu) return false;
    return (await navigator.gpu.requestAdapter()) !== null;
  });
  expect(adapterAvailable).toBe(true);

  await expect
    .poll(async () => {
      const size = await canvas.evaluate((element) => ({
        width: element.width,
        height: element.height,
      }));
      return size.width > 0 && size.height > 0;
    })
    .toBe(true);

  const firstFrame = await canvas.screenshot();
  await expect
    .poll(async () => !(await canvas.screenshot()).equals(firstFrame))
    .toBe(true);

  const oldBackingSize = await canvas.evaluate((element) => ({
    width: element.width,
    height: element.height,
  }));
  await page.setViewportSize({ width: 840, height: 620 });
  await expect
    .poll(async () => {
      const size = await canvas.evaluate((element) => ({
        width: element.width,
        height: element.height,
      }));
      return size.width !== oldBackingSize.width || size.height !== oldBackingSize.height;
    })
    .toBe(true);

  // Pipeline creation can trail the first rendered frame on slower CI runners.
  await page.waitForTimeout(7_500);
  expect(fatalMessages, fatalMessages.join("\n\n")).toEqual([]);
});
