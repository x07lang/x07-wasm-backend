import { mountWebUiApp } from "./app-host.mjs";

const root = document.getElementById("app");

let manifest = null;
try {
  const resp = await fetch("./app.manifest.json", { cache: "no-store" });
  if (resp.ok) manifest = await resp.json();
} catch (_) {}

const wasmUrl = manifest?.wasmUrl ?? manifest?.wasm_url ?? "./app.wasm";
const apiPrefix = manifest?.apiPrefix ?? manifest?.api_prefix ?? null;
const appMeta = manifest?.app ?? null;
const policySnapshotSha256 =
  manifest?.policySnapshotSha256 ?? manifest?.policy_snapshot_sha256 ?? null;

let capabilities = manifest?.capabilities ?? manifest?.caps ?? null;
const capabilitiesUrl = manifest?.capabilitiesUrl ?? manifest?.capabilities_url ?? null;
if (!capabilities && typeof capabilitiesUrl === "string" && capabilitiesUrl) {
  try {
    const resp = await fetch(capabilitiesUrl, { cache: "no-store" });
    if (resp.ok) capabilities = await resp.json();
  } catch (_) {}
}

let componentEsmUrl = null;
if (manifest?.componentEsmUrl || manifest?.component_esm_url) {
  componentEsmUrl = manifest.componentEsmUrl ?? manifest.component_esm_url ?? null;
} else {
  try {
    await import("./transpiled/app.mjs");
    componentEsmUrl = "./transpiled/app.mjs";
  } catch (_) {}
}

const mounted = await mountWebUiApp({
  wasmUrl,
  componentEsmUrl,
  root,
  apiPrefix,
  appMeta,
  capabilities,
  policySnapshotSha256,
});
globalThis.__x07 = mounted;
