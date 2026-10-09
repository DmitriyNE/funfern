// The build the browser runs expect to load: FUNFERN_EXPECT_BUILD when set, as
// CI sets it from the commit it checked out, else this checkout's own commit,
// twelve digits, as build.rs stamps it. The app writes what it was built from
// on the page's root element as `data-funfern-build`; a run holds the two
// equal, so a stale service worker or a cached bundle serving an older
// application fails instead of passing on old code.
import { execSync } from "node:child_process";

export function expectedBuildId() {
  const named = (process.env.FUNFERN_EXPECT_BUILD ?? "").trim();
  if (named) {
    return named;
  }
  return execSync("git rev-parse --short=12 HEAD", { encoding: "utf8" }).trim();
}
