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

// The shaders' own sine and cosine, from the block every shader carries, on the
// page's device against f64 at the same f32 arguments: they hold to 2e-7 where
// WGSL promises its builtins only to 2^-11.
test("the shaders' portable sine and cosine hold to 2e-7 on the page's device", async ({ page }) => {
  await page.route("**/", (route) =>
    route.fulfill({ contentType: "text/html", body: "<!doctype html><title>Trigonometry</title>" }),
  );
  await page.goto("/");
  const source = await readFile("crates/funfern-app/src/canonical_wave.wgsl", "utf8");
  const start = source.indexOf("// Portable trigonometry:");
  const end = source.indexOf("// End of the portable trigonometry.");
  expect(start).toBeGreaterThanOrEqual(0);
  expect(end).toBeGreaterThan(start);
  const block = source.slice(start, end);
  const worst = await page.evaluate(async (block) => {
    const count = 65536;
    const code = `${block}
@group(0) @binding(0) var<storage, read> x: array<f32>;
@group(0) @binding(1) var<storage, read_write> o: array<vec4<f32>>;
@compute @workgroup_size(64) fn main(@builtin(global_invocation_id) id: vec3u) {
  let both = portable_sin_cos(x[id.x]);
  o[id.x] = vec4<f32>(both, reduced_phase(x[id.x]), portable_sin(x[id.x]));
}`;
    const adapter = await navigator.gpu.requestAdapter();
    if (adapter === null) throw new Error("WebGPU adapter unavailable");
    const device = await adapter.requestDevice();
    // Arguments to ten thousand radians, the reduction's whole range of use.
    const arguments_ = new Float32Array(count);
    for (let i = 0; i < count; i += 1) {
      arguments_[i] = Math.sign(i - count / 2) * Math.pow(10, -3 + 7 * Math.abs(i - count / 2) / (count / 2));
    }
    const input = device.createBuffer({ size: count * 4, usage: GPUBufferUsage.STORAGE, mappedAtCreation: true });
    new Float32Array(input.getMappedRange()).set(arguments_);
    input.unmap();
    const output = device.createBuffer({ size: count * 16, usage: GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_SRC });
    const read = device.createBuffer({ size: count * 16, usage: GPUBufferUsage.MAP_READ | GPUBufferUsage.COPY_DST });
    const pipeline = device.createComputePipeline({
      layout: "auto",
      compute: { module: device.createShaderModule({ code }), entryPoint: "main" },
    });
    const group = device.createBindGroup({
      layout: pipeline.getBindGroupLayout(0),
      entries: [input, output].map((buffer, binding) => ({ binding, resource: { buffer } })),
    });
    const encoder = device.createCommandEncoder();
    const pass = encoder.beginComputePass();
    pass.setPipeline(pipeline);
    pass.setBindGroup(0, group);
    pass.dispatchWorkgroups(count / 64);
    pass.end();
    encoder.copyBufferToBuffer(output, 0, read, 0, count * 16);
    device.queue.submit([encoder.finish()]);
    await read.mapAsync(GPUMapMode.READ);
    const values = new Float32Array(read.getMappedRange());
    const worst = { sin: 0, cos: 0, phase: 0 };
    for (let i = 0; i < count; i += 1) {
      const v = arguments_[i];
      worst.sin = Math.max(worst.sin, Math.abs(values[4 * i] - Math.sin(v)), Math.abs(values[4 * i + 3] - Math.sin(v)));
      worst.cos = Math.max(worst.cos, Math.abs(values[4 * i + 1] - Math.cos(v)));
      // Pi and minus pi name the same angle.
      const turn = Math.abs(values[4 * i + 2] - Math.atan2(Math.sin(v), Math.cos(v)));
      worst.phase = Math.max(worst.phase, Math.min(turn, Math.abs(turn - 2 * Math.PI)));
    }
    device.destroy();
    return worst;
  }, block);
  console.log(`portable trigonometry, worst: ${JSON.stringify(worst)}`);
  expect(worst.sin).toBeLessThan(2e-7);
  expect(worst.cos).toBeLessThan(2e-7);
  expect(worst.phase).toBeLessThan(2e-7);
});
