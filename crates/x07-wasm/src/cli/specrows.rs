use std::ffi::OsString;
use std::io::Read as _;

use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::cli::{CliSpecrowsCheckArgs, MachineArgs, Scope};
use crate::diag::{Diagnostic, Severity, Stage};
use crate::report;
use crate::schema::SchemaStore;
use crate::util;

pub fn build_specrows_doc() -> Value {
    let mut doc = json!({
      "schema_version": "x07cli.specrows@0.1.0",
      "app": {
        "name": "x07-wasm",
        "version": env!("CARGO_PKG_VERSION"),
        "about": "x07-wasm: build x07 solve-pure programs to wasm32 and run them deterministically (Phase 0)."
      },
      "rows": [
        ["root","about","Machine-first wasm toolchain for x07 (JSON reports + deterministic runner)."],
        ["root","help","-h","--help","Print help"],
        ["root","version","","--version","Print version"],

        ["root","flag","","--cli-specrows","cli.specrows","Emit deterministic CLI surface table for agents (x07cli.specrows@0.1.0).",{"global":true}],
        ["root","flag","","--json-schema","schema.json","Print the JSON Schema for the selected command scope and exit.",{"global":true}],
        ["root","flag","","--json-schema-id","schema.id","Print the schema id/version string for the selected command scope and exit.",{"global":true}],
        ["root","flag","","--quiet-json","report.quiet-json","Suppress JSON on stdout (use with --report-out).",{"global":true}],
        ["root","flag","","--report-json","report.json-legacy","Hidden alias for --json (compat).",{"global":true,"hidden":true}],

        ["root","opt","","--json","report.json","STR","Emit command report JSON to stdout (values: \"\" or \"pretty\").",{"global":true}],
        ["root","opt","","--report-out","report.out","PATH","Write the same JSON report bytes to a file.",{"global":true}],

        ["app-build","about","Alias for `x07-wasm app build`. Build a full-stack app bundle (frontend web-ui + backend wasi-http component) and emit x07.wasm.app.build.report@0.1.0."],
        ["app-build","flag","","--clean","clean","Delete out-dir before writing bundle artifacts."],
        ["app-build","flag","","--strict","strict","Treat warnings as errors (nonzero exit on any warning)."],
        ["app-build","opt","","--emit","emit","STR","Emit selection: all|frontend|backend|bundle.",{ "default": "all" }],
        ["app-build","opt","","--index","index","PATH","Path to arch/app/index.x07app.json.",{ "default": "arch/app/index.x07app.json" }],
        ["app-build","opt","","--out-dir","out.dir","PATH","Output directory for the app bundle.",{ "default": "dist/app" }],
        ["app-build","opt","","--profile","profile","STR","App profile id to build (from the app index).",{ "default": "app_dev" }],
        ["app-build","opt","","--profile-file","profile.file","PATH","Build using this app profile file directly (bypass index)."],

        ["app-contracts-validate","about","Alias for `x07-wasm app contracts validate`. Validate app contracts (schemas + fixtures) and emit x07.wasm.app.contracts.validate.report@0.1.0."],
        ["app-contracts-validate","flag","","--list","list","List discovered schemas and fixtures and exit (still emits a report)."],
        ["app-contracts-validate","flag","","--strict","strict","Treat warnings as errors."],
        ["app-contracts-validate","opt","","--fixture","fixture","PATH","Validate only specific fixture files (repeatable).",{"multiple":true}],

        ["app-pack","about","Alias for `x07-wasm app pack`. Create a content-addressed app pack from an app bundle and emit x07.wasm.app.pack.report@0.1.0."],
        ["app-pack","opt","","--bundle-manifest","bundle_manifest","PATH","Bundle manifest file produced by x07-wasm app build."],
        ["app-pack","opt","","--out-dir","out_dir","PATH","Output directory for pack."],
        ["app-pack","opt","","--profile-id","profile_id","STR","Pack profile id (used for routing defaults)."],

        ["app-profile-validate","about","Alias for `x07-wasm app profile validate`. Validate arch/app registry + app profiles + cross-references. Emits x07.wasm.app.profile.validate.report@0.1.0."],
        ["app-profile-validate","flag","","--strict","strict","Treat warnings as errors (nonzero exit on any warning)."],
        ["app-profile-validate","opt","","--component-index","component.index","PATH","Path to the wasm component profile registry for cross-checking component_profile_id.",{ "default": "arch/wasm/component/index.x07wasm.component.json" }],
        ["app-profile-validate","opt","","--index","index","PATH","Path to arch/app/index.x07app.json.",{ "default": "arch/app/index.x07app.json" }],
        ["app-profile-validate","opt","","--profile","profile","STR","Validate only this app profile id (from the app index)."],
        ["app-profile-validate","opt","","--profile-file","profile.file","PATH","Validate this app profile file directly (bypass index)."],
        ["app-profile-validate","opt","","--web-ui-index","web_ui.index","PATH","Path to arch/web_ui/index.x07webui.json for cross-checking web_ui_profile_id.",{ "default": "arch/web_ui/index.x07webui.json" }],

        ["app-regress-from-incident","about","Alias for `x07-wasm app regress from-incident`. Generate a regression trace + golden outputs from an incident bundle. Emits x07.wasm.app.regress.from_incident.report@0.1.0."],
        ["app-regress-from-incident","flag","","--dry-run","dry.run","Do not write files; validate and emit report only."],
        ["app-regress-from-incident","flag","","--strict","strict","Treat warnings as errors (nonzero exit on any warning)."],
        ["app-regress-from-incident","opt","","--name","name","STR","Base name for generated case files.",{ "default": "incident" }],
        ["app-regress-from-incident","opt","","--out-dir","out.dir","PATH","Output directory for generated regression assets.",{ "default": "tests/regress" }],
        ["app-regress-from-incident","arg","INCIDENT_DIR","incident.dir","Path to an app incident bundle directory.",{ "required": true }],

        ["binding-resolve","about","Alias for `x07-wasm binding resolve`. Normalize provider-neutral workload binding requirements and emit x07.wasm.workload.surface.report@0.1.0."],
        ["binding-resolve","opt","","--manifest","manifest","PATH","Path to arch/service/index.x07service.json.",{ "default": "arch/service/index.x07service.json" }],
        ["binding-resolve","opt","","--pack-manifest","pack_manifest","PATH","Workload pack manifest file to inspect instead of source inputs."],
        ["binding-resolve","opt","","--project","project","PATH","Path to x07 project manifest.",{ "default": "x07.json" }],

        ["app-serve","about","Alias for `x07-wasm app serve`. Serve a built app bundle: static frontend + /api routed to backend component. Emits x07.wasm.app.serve.report@0.1.0."],
        ["app-serve","flag","","--strict","strict","Treat warnings as errors (nonzero exit on any warning)."],
        ["app-serve","flag","","--strict-mime","strict.mime","Fail if .wasm is not served as application/wasm (exact, no parameters)."],
        ["app-serve","opt","","--addr","addr","STR","Bind address in host:port form. Port 0 selects an ephemeral port.",{ "default": "127.0.0.1:0" }],
        ["app-serve","opt","","--api-prefix","api.prefix","STR","API route prefix override (default comes from app profile).",{ "default": "/api" }],
        ["app-serve","opt","","--dir","dir","PATH","Directory containing the app bundle (default: dist/app).",{ "default": "dist/app" }],
        ["app-serve","opt","","--mode","mode","STR","Serve mode: listen|smoke|canary.",{ "default": "listen" }],
        ["app-serve","opt","","--ops","ops","PATH","Ops profile file (x07.app.ops.profile@0.1.0) for capability enforcement."],

        ["app-test","about","Alias for `x07-wasm app test`. Run deterministic E2E trace replay (UI dispatch + backend HTTP exchanges) and emit x07.wasm.app.test.report@0.1.0."],
        ["app-test","flag","","--strict","strict","Treat warnings as errors (nonzero exit on any warning)."],
        ["app-test","flag","","--update-golden","update.golden","Update golden outputs from current outputs."],
        ["app-test","opt","","--dir","dir","PATH","Directory containing the app bundle (default: dist/app).",{ "default": "dist/app" }],
        ["app-test","opt","","--max-steps","max.steps","U32","Maximum number of trace steps to replay.",{ "default": 10000 }],
        ["app-test","opt","","--trace","trace","PATH","Path to x07.app.trace@... JSON to replay.",{ "required": true }],

        ["app-verify","about","Alias for `x07-wasm app verify`. Verify an app pack (digests + required headers) and emit x07.wasm.app.verify.report@0.1.0."],
        ["app-verify","opt","","--pack-manifest","pack_manifest","PATH","Pack manifest file (x07.app.pack@0.1.0)."],

        ["build","about","Build an x07 project to a wasm32 reactor module (exports x07_solve_v2)."],
        ["build","flag","","--no-manifest","manifest.none","Do not write the artifact manifest file."],

        ["build","opt","","--artifact-out","artifact.out","PATH","Artifact manifest output path."],
        ["build","opt","","--check-exports","exports.check","STR","Validate required exports exist (true/false; default true)."],
        ["build","opt","","--codegen-backend","codegen.backend","STR","Override the profile’s codegen_backend."],
        ["build","opt","","--emit-dir","emit.dir","PATH","Directory for intermediate artifacts."],
        ["build","opt","","--index","index","PATH","Path to wasm profile registry (default: arch/wasm/index.x07wasm.json)."],
        ["build","opt","","--out","out","PATH","Output wasm path."],
        ["build","opt","","--profile","profile.id","STR","Profile id (loaded from arch/wasm/index.x07wasm.json)."],
        ["build","opt","","--profile-file","profile.file","PATH","Validate and use this profile JSON file directly (bypass registry)."],
        ["build","opt","","--project","project","PATH","Path to x07 project manifest (default: x07.json)."],

        ["caps-validate","about","Alias for `x07-wasm caps validate`. Validate capabilities profile and emit x07.wasm.caps.validate.report@0.1.0."],
        ["caps-validate","opt","","--profile","profile","PATH","Capabilities profile file (x07.app.capabilities@0.2.0)."],

        ["cli-specrows-check","about","Validate x07-wasm --cli-specrows output against x07cli.specrows@0.1.0 + invariants. Alias: `x07-wasm cli specrows check` / `x07-wasm cli validate-specrows`."],
        ["cli-specrows-check","flag","","--stdin","stdin","Read specrows JSON from stdin (mutually exclusive with --in)."],
        ["cli-specrows-check","opt","","--expect-app-name","expect.app.name","STR","Expected app.name (default: x07-wasm).",{"required":false}],
        ["cli-specrows-check","opt","","--in","in","PATH","Read specrows JSON from file (mutually exclusive with --stdin; default is self).",{"required":false}],

        ["component-build","about","Build an x07 project into an x07:solve component (and optional adapters). Alias: `x07-wasm component build`."],
        ["component-build","flag","","--clean","clean","Delete out-dir before building."],
        ["component-build","opt","","--emit","emit","STR","Artifact set to emit: solve|http|http-state-doc|cli|http-native|cli-native|http-adapter|cli-adapter|all (default: all)."],
        ["component-build","opt","","--index","index","PATH","Path to the component profile registry (default: arch/wasm/component/index.x07wasm.component.json)."],
        ["component-build","opt","","--out-dir","out.dir","PATH","Output directory for component artifacts (default: target/x07-wasm/component)."],
        ["component-build","opt","","--profile","profile.id","STR","Component profile id (loaded from arch/wasm/component/index.x07wasm.component.json)."],
        ["component-build","opt","","--profile-file","profile.file","PATH","Validate and use this component profile JSON file directly (bypass registry)."],
        ["component-build","opt","","--project","project","PATH","Path to x07 project manifest (default: x07.json)."],
        ["component-build","opt","","--wasm-index","wasm.index","PATH","Path to wasm profile registry (default: arch/wasm/index.x07wasm.json)."],
        ["component-build","opt","","--wasm-profile","wasm.profile.id","STR","WASM profile id (loaded from arch/wasm/index.x07wasm.json)."],
        ["component-build","opt","","--wasm-profile-file","wasm.profile.file","PATH","Validate and use this wasm profile JSON file directly (bypass registry)."],

            ["component-compose","about","Compose adapter component with solve component (wac plug) to produce runnable standard-world components. Alias: `x07-wasm component compose`."],
            ["component-compose","flag","","--targets-check","targets.check","Also run a targets check on the output component."],
            ["component-compose","opt","","--adapter","adapter","STR","Adapter kind: http|http-state-doc|cli (alias: --target)."],
            ["component-compose","opt","","--adapter-component","adapter.component","PATH","Path to adapter component (.wasm)."],
            ["component-compose","opt","","--artifact-out","artifact.out","PATH","Artifact manifest output path."],
            ["component-compose","opt","","--out","out","PATH","Output path for composed component (.wasm)."],
            ["component-compose","opt","","--solve","solve","PATH","Path to solve component (.wasm)."],

            ["component-profile-validate","about","Validate arch/wasm/component/index.x07wasm.component.json and referenced component profile files. Alias: `x07-wasm component profile validate`."],
            ["component-profile-validate","flag","","--strict","strict","Treat warnings as errors."],
            ["component-profile-validate","opt","","--index","index","PATH","Path to the component profile registry (default: arch/wasm/component/index.x07wasm.component.json)."],
            ["component-profile-validate","opt","","--profile","profile.id","STR","Only validate specific profile id(s).",{"multiple":true}],

            ["component-run","about","Run a wasi:cli/command component (export run) and emit a machine report. Alias: `x07-wasm component run`."],
            ["component-run","opt","","--args-json","args.json","STR","Process args as JSON array of strings."],
            ["component-run","opt","","--component","component","PATH","Path to component wasm to run."],
            ["component-run","opt","","--incidents-dir","incidents.dir","PATH","Root directory for incident bundles (default: .x07-wasm/incidents)."],
            ["component-run","opt","","--max-output-bytes","output.max.bytes","U32","Hard cap on stdout/stderr bytes captured by the host."],
            ["component-run","opt","","--max-wall-ms","wall.max.ms","U32","Hard cap on wall time spent running the component (ms).",{"required":false}],
            ["component-run","opt","","--stderr-out","stderr.out","PATH","Write stderr bytes to a file.",{"required":false}],
            ["component-run","opt","","--stdin","stdin","PATH","Stdin bytes file path (mutually exclusive with --stdin-b64).",{"required":false}],
            ["component-run","opt","","--stdin-b64","stdin.b64","STR","Stdin bytes as base64 (mutually exclusive with --stdin).",{"required":false}],
            ["component-run","opt","","--stdout-out","stdout.out","PATH","Write stdout bytes to a file.",{"required":false}],

            ["component-targets","about","Check that a component targets a given WIT world (wac targets). Alias: `x07-wasm component targets`."],
            ["component-targets","flag","","--strict","strict","Treat warnings as errors."],
            ["component-targets","opt","","--component","component","PATH","Path to component wasm to check."],
            ["component-targets","opt","","--wit","wit","PATH","Path to a .wit file containing the world to target."],
            ["component-targets","opt","","--world","world","STR","World name within the WIT file."],

            ["deploy-plan","about","Alias for `x07-wasm deploy plan`. Generate progressive delivery plan from pack + ops profile and emit x07.wasm.deploy.plan.report@0.1.0."],
            ["deploy-plan","opt","","--ops","ops","PATH","Ops profile file (x07.app.ops.profile@0.1.0)."],
            ["deploy-plan","opt","","--emit-k8s","emit.k8s","STR","Emit Kubernetes YAML outputs (true/false; default true)."],
            ["deploy-plan","opt","","--out-dir","out_dir","PATH","Output directory for deploy plan + emitted manifests."],
            ["deploy-plan","opt","","--environment-id","environment_id","STR","Optional environment id for emitted telemetry identity labels."],
            ["deploy-plan","opt","","--deployment-id","deployment_id","STR","Optional deployment id for emitted telemetry identity labels."],
            ["deploy-plan","opt","","--service-id","service_id","STR","Optional service id for emitted telemetry identity labels."],
            ["deploy-plan","opt","","--pack-manifest","pack_manifest","PATH","App pack manifest file (x07.app.pack@0.1.0)."],

        ["device-build","about","Alias for `x07-wasm device build`. Build a device UI bundle (web-ui reducer wasm + pinned host ABI) and emit x07.wasm.device.build.report@0.1.0."],
        ["device-build","flag","","--clean","clean","Delete out-dir before writing bundle artifacts."],
        ["device-build","flag","","--strict","strict","Treat warnings as errors (nonzero exit on any warning)."],
        ["device-build","opt","","--index","index","PATH","Path to arch/device/index.x07device.json.",{ "default": "arch/device/index.x07device.json" }],
        ["device-build","opt","","--out-dir","out.dir","PATH","Output directory for the device bundle.",{ "default": "dist/device" }],
        ["device-build","opt","","--profile","profile.id","STR","Device profile id to build (from the device index)."],
        ["device-build","opt","","--profile-file","profile.file","PATH","Build using this device profile file directly (bypass index)."],

        ["device-index-validate","about","Alias for `x07-wasm device index validate`. Validate device profile registry and emit x07.wasm.device.index.validate.report@0.1.0."],
        ["device-index-validate","opt","","--index","index","PATH","Path to arch/device/index.x07device.json.",{ "default": "arch/device/index.x07device.json" }],

        ["device-package","about","Alias for `x07-wasm device package`. Package a device bundle into a target payload (desktop app bundle or iOS/Android project) and emit x07.wasm.device.package.report@0.2.0."],
        ["device-package","opt","","--bundle","bundle.dir","PATH","Directory containing the device bundle.",{ "default": "dist/device" }],
        ["device-package","opt","","--out-dir","out.dir","PATH","Output directory for the packaged payload + package.manifest.json.",{ "default": "dist/device_package" }],
        ["device-package","opt","","--target","target","STR","Device target (`desktop`, `ios`, `android`).",{ "default": "desktop" }],

        ["device-regress-from-incident","about","Alias for `x07-wasm device regress from-incident`. Convert a captured device incident into deterministic regression fixtures and emit x07.wasm.device.regress.from_incident.report@0.2.0."],
        ["device-regress-from-incident","flag","","--dry-run","dry.run","Do not write files; validate and emit report only."],
        ["device-regress-from-incident","flag","","--strict","strict","Treat warnings as errors (nonzero exit on any warning)."],
        ["device-regress-from-incident","opt","","--name","name","STR","Base name for generated case files.",{ "default": "device_incident" }],
        ["device-regress-from-incident","opt","","--out-dir","out.dir","PATH","Output directory for generated regression assets.",{ "default": "tests/regress" }],
        ["device-regress-from-incident","arg","INCIDENT_DIR","incident.dir","Path to a device incident bundle directory.",{ "required": true }],

        ["device-provenance-attest","about","Alias for `x07-wasm device provenance attest`. Create SLSA provenance attestation for a device bundle and emit x07.wasm.device.provenance.attest.report@0.1.0."],
        ["device-provenance-attest","opt","","--bundle-dir","bundle.dir","PATH","Directory containing the device bundle.",{ "default": "dist/device" }],
        ["device-provenance-attest","opt","","--out","out","PATH","Output attestation file."],
        ["device-provenance-attest","opt","","--predicate-type","predicate_type","STR","In-toto Statement predicateType."],
        ["device-provenance-attest","opt","","--signing-key","signing_key","PATH","Ed25519 signing key seed file (base64, 32 bytes)."],

        ["device-provenance-verify","about","Alias for `x07-wasm device provenance verify`. Verify SLSA provenance attestation against current device bundle artifacts and emit x07.wasm.device.provenance.verify.report@0.1.0."],
        ["device-provenance-verify","opt","","--attestation","attestation","PATH","DSSE envelope file (x07.provenance.dsse.envelope@0.1.0)."],
        ["device-provenance-verify","opt","","--bundle-dir","bundle.dir","PATH","Directory containing the device bundle referenced by the attestation.",{ "default": "dist/device" }],
        ["device-provenance-verify","opt","","--trusted-public-key","trusted_public_key","PATH","Trusted Ed25519 public key file (base64, 32 bytes)."],

        ["device-profile-validate","about","Alias for `x07-wasm device profile validate`. Validate device profiles and cross-references. Emits x07.wasm.device.profile.validate.report@0.1.0."],
        ["device-profile-validate","flag","","--strict","strict","Treat warnings as errors (nonzero exit on any warning)."],
        ["device-profile-validate","opt","","--index","index","PATH","Path to arch/device/index.x07device.json.",{ "default": "arch/device/index.x07device.json" }],
        ["device-profile-validate","opt","","--profile","profile.id","STR","Only validate specific profile id(s).",{"multiple":true}],
        ["device-profile-validate","opt","","--profile-file","profile.file","PATH","Validate this device profile file directly (bypass index)."],

        ["device-run","about","Alias for `x07-wasm device run`. Run a device UI bundle using the system WebView host and emit x07.wasm.device.run.report@0.1.0."],
        ["device-run","flag","","--headless-smoke","headless.smoke","Ask the host to exit quickly after the UI becomes ready (smoke mode)."],
        ["device-run","opt","","--bundle","bundle.dir","PATH","Directory containing the device bundle.",{ "default": "dist/device" }],
        ["device-run","opt","","--target","target","STR","Device target (currently only `desktop` is supported).",{ "default": "desktop" }],

        ["device-verify","about","Alias for `x07-wasm device verify`. Verify a device bundle against its manifest + pinned host ABI and emit x07.wasm.device.verify.report@0.2.0."],
        ["device-verify","opt","","--dir","dir","PATH","Directory containing the device bundle.",{ "default": "dist/device" }],

        ["doctor","about","Check wasm toolchain prerequisites and emit a machine report."],

        ["http-contracts-validate","about","Alias for `x07-wasm http contracts validate`. Validate http reducer schema set + fixtures and emit x07.wasm.http.contracts.validate.report@0.1.0."],
        ["http-contracts-validate","flag","","--strict","strict","Fail if any fixture/schema check fails."],

        ["http-regress-from-incident","about","Alias for `x07-wasm http regress from-incident`. Generate a regression test + trace fixture from an incident bundle and emit x07.wasm.http.regress.from.incident.report@0.1.0."],
        ["http-regress-from-incident","opt","","--incident-dir","incident_dir","PATH","Incident bundle directory."],
        ["http-regress-from-incident","opt","","--out-dir","out_dir","PATH","Where to write generated test/fixture.",{ "default": "tests/regress" }],

        ["topology-preview","about","Alias for `x07-wasm topology preview`. Preview workload grouping and placement for a service-oriented workload and emit x07.wasm.workload.surface.report@0.1.0."],
        ["topology-preview","opt","","--manifest","manifest","PATH","Path to arch/service/index.x07service.json.",{ "default": "arch/service/index.x07service.json" }],
        ["topology-preview","opt","","--pack-manifest","pack_manifest","PATH","Workload pack manifest file to inspect instead of source inputs."],
        ["topology-preview","opt","","--profile","profile","STR","Only emit the named topology profile."],
        ["topology-preview","opt","","--project","project","PATH","Path to x07 project manifest.",{ "default": "x07.json" }],

        ["workload-build","about","Alias for `x07-wasm workload build`. Emit deterministic workload metadata documents from a service manifest and emit x07.wasm.workload.surface.report@0.1.0."],
        ["workload-build","opt","","--manifest","manifest","PATH","Path to arch/service/index.x07service.json.",{ "default": "arch/service/index.x07service.json" }],
        ["workload-build","opt","","--out-dir","out.dir","PATH","Output directory for generated workload documents.",{ "default": "dist/workload-build" }],
        ["workload-build","opt","","--project","project","PATH","Path to x07 project manifest.",{ "default": "x07.json" }],

        ["workload-contracts-validate","about","Alias for `x07-wasm workload contracts validate`. Validate generated workload documents against public platform contracts and emit x07.wasm.workload.surface.report@0.1.0."],
        ["workload-contracts-validate","opt","","--manifest","manifest","PATH","Path to arch/service/index.x07service.json.",{ "default": "arch/service/index.x07service.json" }],
        ["workload-contracts-validate","opt","","--pack-manifest","pack_manifest","PATH","Workload pack manifest file to validate instead of source inputs."],
        ["workload-contracts-validate","opt","","--profile","profile","STR","Only validate the named topology profile."],
        ["workload-contracts-validate","opt","","--project","project","PATH","Path to x07 project manifest.",{ "default": "x07.json" }],
        ["workload-contracts-validate","opt","","--schema-dir","schema.dir","PATH","Path to x07-platform-contracts/spec/schemas."],

        ["workload-inspect","about","Alias for `x07-wasm workload inspect`. Inspect a workload pack deterministically and emit x07.wasm.workload.surface.report@0.1.0."],
        ["workload-inspect","opt","","--pack-manifest","pack_manifest","PATH","Workload pack manifest file.",{ "default": "dist/workload/workload.pack.json" }],
        ["workload-inspect","opt","","--view","view","STR","Inspection view: summary|full.",{ "default": "full" }],

        ["workload-pack","about","Alias for `x07-wasm workload pack`. Emit workload documents, a source snapshot pack directory, and a deployable `x07.workload.pack@0.1.0` manifest from a service manifest; emit x07.wasm.workload.surface.report@0.1.0."],
        ["workload-pack","opt","","--manifest","manifest","PATH","Path to arch/service/index.x07service.json.",{ "default": "arch/service/index.x07service.json" }],
        ["workload-pack","opt","","--out-dir","out.dir","PATH","Output directory for the workload pack.",{ "default": "dist/workload" }],
        ["workload-pack","opt","","--project","project","PATH","Path to x07 project manifest.",{ "default": "x07.json" }],
        ["workload-pack","opt","","--runtime-image","runtime.image","STR","OCI image reference to attach to `native-http` workload cells for runtime deployment."],
        ["workload-pack","opt","","--container-port","container.port","U32","Container port recorded for attached OCI image executables.",{ "default": 8080 }],

        ["http-serve","about","Alias for `x07-wasm http serve`. Run an http reducer effect loop and emit x07.wasm.http.serve.report@0.1.0."],
        ["http-serve","opt","","--component","component","PATH","Reducer component wasm."],
        ["http-serve","opt","","--max-effect-results-bytes","max_effect_results_bytes","U32","Max total effect result bytes.",{ "default": 1048576 }],
        ["http-serve","opt","","--max-effect-steps","max_effect_steps","U32","Max dispatch/frame iterations.",{ "default": 64 }],
        ["http-serve","opt","","--max-fuel","max_fuel","U32","Max Wasmtime fuel (overrides profile)."],
        ["http-serve","opt","","--mode","mode","STR","canary|listen.",{ "default": "listen" }],
        ["http-serve","opt","","--ops","ops","PATH","Ops profile file (x07.app.ops.profile@0.1.0) for capability enforcement."],

        ["http-test","about","Alias for `x07-wasm http test`. Run http reducer trace replays and emit x07.wasm.http.test.report@0.1.0."],
        ["http-test","opt","","--component","component","PATH","Reducer component wasm."],
        ["http-test","opt","","--trace","trace","PATH","Trace case file(s) to replay.",{"multiple":true}],

        ["ops-validate","about","Alias for `x07-wasm ops validate`. Validate ops profile and referenced capability/policy/SLO inputs; emit x07.wasm.ops.validate.report@0.1.0."],
        ["ops-validate","opt","","--index","index","PATH","Ops index file (x07.arch.app.ops.index@0.1.0).",{ "default": "arch/app/ops/index.x07ops.json" }],
        ["ops-validate","opt","","--profile","profile","PATH","Ops profile file (x07.app.ops.profile@0.1.0)."],
        ["ops-validate","opt","","--profile-id","profile_id","STR","Ops profile id resolved via arch/app/ops/index.x07ops.json."],

        ["policy-validate","about","Alias for `x07-wasm policy validate`. Validate policy card files and emit x07.wasm.policy.validate.report@0.1.0."],
        ["policy-validate","flag","","--strict","strict","Fail if any policy card fails validation."],
        ["policy-validate","opt","","--card","card","PATH","Policy card file (x07.policy.card@0.1.0). May be repeated.",{"multiple":true}],
        ["policy-validate","opt","","--cards-dir","cards_dir","PATH","Directory of policy cards to validate."],

        ["profile-validate","about","Validate arch/wasm/index.x07wasm.json and referenced profile files. Alias: `x07-wasm profile validate`."],
        ["profile-validate","opt","","--index","index","PATH","Path to wasm profile registry (default: arch/wasm/index.x07wasm.json).",{"required":false}],
        ["profile-validate","opt","","--profile","profile.id","STR","Validate only this profile id (looked up in the registry).",{"required":false}],
        ["profile-validate","opt","","--profile-file","profile.file","PATH","Validate a profile JSON file directly (bypass registry).",{"required":false}],

        ["provenance-attest","about","Alias for `x07-wasm provenance attest`. Create SLSA provenance attestation for an app pack and emit x07.wasm.provenance.attest.report@0.1.0."],
        ["provenance-attest","opt","","--ops","ops","PATH","Ops profile file (x07.app.ops.profile@0.1.0)."],
        ["provenance-attest","opt","","--out","out","PATH","Output attestation file."],
        ["provenance-attest","opt","","--pack-manifest","pack_manifest","PATH","App pack manifest file (x07.app.pack@0.1.0)."],
        ["provenance-attest","opt","","--predicate-type","predicate_type","STR","In-toto Statement predicateType."],
        ["provenance-attest","opt","","--signing-key","signing_key","PATH","Ed25519 signing key seed file (base64, 32 bytes)."],

        ["provenance-verify","about","Alias for `x07-wasm provenance verify`. Verify SLSA provenance attestation against current artifacts and emit x07.wasm.provenance.verify.report@0.1.0."],
        ["provenance-verify","opt","","--attestation","attestation","PATH","DSSE envelope file (x07.provenance.dsse.envelope@0.1.0)."],
        ["provenance-verify","opt","","--pack-dir","pack_dir","PATH","Directory containing the packed assets referenced by the attestation."],
        ["provenance-verify","opt","","--trusted-public-key","trusted_public_key","PATH","Trusted Ed25519 public key file (base64, 32 bytes)."],

        ["run","about","Run a wasm module exporting x07_solve_v2 under Wasmtime; emit output bytes + JSON report."],
        ["run","opt","","--arena-cap-bytes","arena.cap.bytes","U32","Arena capacity passed to x07_solve_v2 (bytes)."],
        ["run","opt","","--index","index","PATH","Path to wasm profile registry (default: arch/wasm/index.x07wasm.json)."],
        ["run","opt","","--input","input","PATH","Input bytes file path (mutually exclusive with --input-hex/--input-base64)."],
        ["run","opt","","--input-base64","input.base64","STR","Input bytes as base64 (mutually exclusive with --input/--input-hex)."],
        ["run","opt","","--input-hex","input.hex","BYTES_HEX","Input bytes as hex (mutually exclusive with --input/--input-base64)."],
        ["run","opt","","--max-output-bytes","output.max.bytes","U32","Hard cap enforced on returned bytes_t.len."],
        ["run","opt","","--output-out","output.out","PATH","Write output bytes to a file."],
        ["run","opt","","--profile","profile.id","STR","Profile id (for defaults like arena/max-output)."],
            ["run","opt","","--profile-file","profile.file","PATH","Validate and use this profile JSON file directly (bypass registry)."],
            ["run","opt","","--wasm","wasm","PATH","Path to wasm module."],

            ["serve","about","Run a wasi:http/proxy component as a local canary and emit a machine-readable serve report."],
            ["serve","opt","","--addr","addr","STR","Listen address for mode=listen (e.g., 127.0.0.1:8080).",{"required":false}],
            ["serve","opt","","--component","component","PATH","Path to HTTP component (.wasm)."],
            ["serve","opt","","--incidents-dir","incidents.dir","PATH","Root directory for incident bundles (default: .x07-wasm/incidents)."],
            ["serve","opt","","--max-concurrent","concurrency.max","U32","Hard cap on concurrent request handling."],
            ["serve","opt","","--max-request-bytes","request.max.bytes","U32","Hard cap on request bytes (body + headers)."],
            ["serve","opt","","--max-response-bytes","response.max.bytes","U32","Hard cap on response body bytes."],
            ["serve","opt","","--max-wall-ms-per-request","wall.max.ms.per.request","U32","Hard cap on wall time spent per request (ms)."],
            ["serve","opt","","--method","method","STR","Request method for canary mode.",{"required":false}],
            ["serve","opt","","--mode","mode","STR","Mode: canary|listen.",{"required":false}],
            ["serve","opt","","--ops","ops","PATH","Ops profile file (x07.app.ops.profile@0.1.0) for capability enforcement."],
            ["serve","opt","","--path","path","STR","Request path for canary mode.",{"required":false}],
            ["serve","opt","","--request-body","request.body","BYTES","Request body bytes for canary mode (hex:, b64:, @path).",{"required":false}],
            ["serve","opt","","--stop-after","stop.after","U32","Stop after N requests (canary mode; or listen mode if nonzero).",{"required":false}],

        ["slo-eval","about","Alias for `x07-wasm slo eval`. Evaluate SLO profile against a metrics snapshot and emit x07.wasm.slo.eval.report@0.1.0."],
        ["slo-eval","opt","","--metrics","metrics","PATH","Metrics snapshot file (x07.metrics.snapshot@0.1.0)."],
        ["slo-eval","opt","","--profile","profile","PATH","SLO profile file (x07.slo.profile@0.1.0)."],

        ["slo-validate","about","Alias for `x07-wasm slo validate`. Validate SLO profile file and emit x07.wasm.slo.validate.report@0.1.0."],
        ["slo-validate","opt","","--profile","profile","PATH","SLO profile file (x07.slo.profile@0.1.0)."],

        ["toolchain-validate","about","Alias for `x07-wasm toolchain validate`. Validate pinned toolchain requirements (versions + probes) and emit x07.wasm.toolchain.validate.report@0.1.0."],
        ["toolchain-validate","opt","","--index","index","PATH","Toolchain index file (x07.arch.wasm.toolchain.index@0.1.0).",{ "default": "arch/wasm/toolchain/index.x07wasm.toolchain.json" }],
        ["toolchain-validate","opt","","--profile","profile","PATH","Toolchain profile file (x07.wasm.toolchain.profile@0.1.0)."],
        ["toolchain-validate","opt","","--profile-id","profile_id","STR","Toolchain profile id resolved via arch/wasm/toolchain/index.x07wasm.toolchain.json."],

        ["web-ui-build","about","Build a browser-runnable web-ui bundle (core wasm or component+ESM). Alias: `x07-wasm web-ui build`."],
        ["web-ui-build","flag","","--clean","clean","Delete out-dir before writing artifacts."],
        ["web-ui-build","flag","","--strict","strict","Treat warnings as errors."],
        ["web-ui-build","opt","","--format","format","STR","Override build output format: core|component (default comes from the web-ui profile).",{"required":false}],
        ["web-ui-build","opt","","--index","index","PATH","Path to the web-ui profile registry (default: arch/web_ui/index.x07webui.json).",{"required":false}],
        ["web-ui-build","opt","","--out-dir","out.dir","PATH","Output directory for dist artifacts (default: dist).",{"required":false}],
        ["web-ui-build","opt","","--profile","profile.id","STR","Web UI profile id (loaded from arch/web_ui/index.x07webui.json).",{"required":false}],
        ["web-ui-build","opt","","--profile-file","profile.file","PATH","Validate and use this web-ui profile JSON file directly (bypass registry).",{"required":false}],
        ["web-ui-build","opt","","--project","project","PATH","Path to x07 project manifest (default: x07.json).",{"required":false}],
        ["web-ui-build","opt","","--wasm-index","wasm.index","PATH","Path to wasm profile registry (default: arch/wasm/index.x07wasm.json).",{"required":false}],

        ["web-ui-contracts-validate","about","Validate web-ui contracts (schemas + fixtures) and emit a machine report. Alias: `x07-wasm web-ui contracts validate`."],
        ["web-ui-contracts-validate","flag","","--list","list","List discovered schemas and fixtures and exit (still emits a report)."],
        ["web-ui-contracts-validate","flag","","--strict","strict","Treat warnings as errors."],
        ["web-ui-contracts-validate","opt","","--fixture","fixture","PATH","Validate only specific fixture files (repeatable).",{"multiple":true}],

        ["web-ui-profile-validate","about","Validate arch/web_ui/index.x07webui.json and referenced web-ui profile files. Alias: `x07-wasm web-ui profile validate`."],
        ["web-ui-profile-validate","flag","","--strict","strict","Treat warnings as errors."],
        ["web-ui-profile-validate","opt","","--index","index","PATH","Path to the web-ui profile registry (default: arch/web_ui/index.x07webui.json).",{"required":false}],
        ["web-ui-profile-validate","opt","","--profile","profile.id","STR","Only validate specific profile id(s).",{"multiple":true}],
        ["web-ui-profile-validate","opt","","--profile-file","profile.file","PATH","Validate and use this web-ui profile JSON file directly (bypass registry).",{"required":false}],

        ["web-ui-regress-from-incident","about","Convert a captured web-ui incident.json into deterministic regression fixtures. Alias: `x07-wasm web-ui regress-from-incident` / `x07-wasm web-ui regress from-incident`."],
        ["web-ui-regress-from-incident","flag","","--strict","strict","Treat warnings as errors."],
        ["web-ui-regress-from-incident","opt","","--incident","incident","PATH","Path to incident artifact JSON captured by the web-ui host."],
        ["web-ui-regress-from-incident","opt","","--name","name","STR","Base name for generated case files (default: incident).",{"required":false}],
        ["web-ui-regress-from-incident","opt","","--out-dir","out.dir","PATH","Output directory for generated regression assets (default: tests/regress).",{"required":false}],

        ["web-ui-serve","about","Serve a web-ui dist directory with correct wasm MIME. Alias: `x07-wasm web-ui serve`."],
        ["web-ui-serve","flag","","--strict-mime","mime.strict","Fail if .wasm is not served as application/wasm."],
        ["web-ui-serve","opt","","--addr","addr","STR","Bind address (host:port). Port 0 selects an ephemeral port (default: 127.0.0.1:0).",{"required":false}],
        ["web-ui-serve","opt","","--dir","dir","PATH","Directory to serve (default: dist).",{"required":false}],
        ["web-ui-serve","opt","","--incidents-dir","incidents.dir","PATH","Root directory for incident bundles (default: .x07-wasm/incidents).",{"required":false}],
        ["web-ui-serve","opt","","--mode","mode","STR","Serve mode: listen|smoke (default: listen).",{"required":false}],

        ["web-ui-test","about","Replay web-ui trace cases and emit a machine test report. Alias: `x07-wasm web-ui test`."],
        ["web-ui-test","flag","","--strict","strict","Treat warnings as errors."],
        ["web-ui-test","flag","","--update-golden","update.golden","Update trace fixtures in-place from current outputs."],
        ["web-ui-test","opt","","--case","case","PATH","Trace case file(s) to replay (repeatable).",{"multiple":true}],
        ["web-ui-test","opt","","--dist-dir","dist.dir","PATH","Directory containing built dist artifacts (default: dist).",{"required":false}],
        ["web-ui-test","opt","","--incidents-dir","incidents.dir","PATH","Root directory for incident bundles (default: .x07-wasm/incidents).",{"required":false}],
        ["web-ui-test","opt","","--max-steps","steps.max","U32","Maximum number of trace steps to replay per case."],

            ["wit-validate","about","Validate arch/wit/index.x07wit.json and all referenced WIT packages (offline)."],
            ["wit-validate","flag","","--list","list","List packages discovered in the registry and exit (still emits a report)."],
            ["wit-validate","flag","","--strict","strict","Treat warnings as errors."],
        ["wit-validate","opt","","--index","index","PATH","Path to the WIT registry file (default: arch/wit/index.x07wit.json)."],
        ["wit-validate","opt","","--package","package","STR","Only validate specific package id(s), e.g. wasi:http@0.2.8.",{"multiple":true}]
      ]
    });

    if let Some(rows) = doc.get_mut("rows").and_then(Value::as_array_mut) {
        rows.sort_by_key(canonical_row_key);
    }

    doc
}

pub fn cmd_cli_specrows_check(
    raw_argv: &[OsString],
    scope: Scope,
    machine: &MachineArgs,
    args: CliSpecrowsCheckArgs,
) -> Result<u8> {
    let started = std::time::Instant::now();

    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    let mut meta = report::meta::tool_meta(raw_argv, started);
    meta.nondeterminism.uses_process = false;
    meta.nondeterminism.uses_os_time = false;
    meta.nondeterminism.uses_network = false;

    let (mode, input_digest, bytes) = if args.stdin {
        let mut buf = Vec::new();
        match std::io::stdin().read_to_end(&mut buf) {
            Ok(_) => ("stdin_v1", None, buf),
            Err(err) => {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_SPECROWS_STDIN_IO_FAILED",
                    Severity::Error,
                    Stage::Run,
                    format!("failed to read stdin: {err}"),
                ));
                ("stdin_v1", None, Vec::new())
            }
        }
    } else if let Some(path) = &args.r#in {
        let mut digest = report::meta::FileDigest {
            path: path.display().to_string(),
            sha256: "0".repeat(64),
            bytes_len: 0,
        };
        let bytes = match std::fs::read(path) {
            Ok(b) => {
                digest.sha256 = util::sha256_hex(&b);
                digest.bytes_len = b.len() as u64;
                b
            }
            Err(err) => {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_SPECROWS_INPUT_READ_FAILED",
                    Severity::Error,
                    Stage::Parse,
                    format!("failed to read input {}: {err}", path.display()),
                ));
                Vec::new()
            }
        };
        meta.inputs.push(digest.clone());
        ("file_v1", Some(digest), bytes)
    } else {
        (
            "self_v1",
            None,
            report::canon::canonical_json_bytes(&build_specrows_doc())?,
        )
    };

    let mut parsed_ok = true;
    let doc: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(err) => {
            parsed_ok = false;
            diagnostics.push(Diagnostic::new(
                "X07WASM_SPECROWS_PARSE_FAILED",
                Severity::Error,
                Stage::Parse,
                format!("failed to parse JSON: {err}"),
            ));
            json!(null)
        }
    };

    let schema_id = "https://x07.org/spec/x07cli.specrows.schema.json";
    let store = SchemaStore::new()?;

    let mut schema_valid = false;
    if parsed_ok {
        match store.validate(schema_id, &doc) {
            Ok(diags) => {
                if diags.is_empty() {
                    schema_valid = true;
                } else {
                    diagnostics.extend(diags);
                }
            }
            Err(err) => {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_SCHEMA_VALIDATE_FAILED",
                    Severity::Error,
                    Stage::Run,
                    format!("{err:#}"),
                ));
            }
        }
    }

    let mut rows_count = 0u64;
    let mut scopes: Vec<String> = Vec::new();
    let mut app_name: Option<String> = None;
    let mut app_version: Option<String> = None;
    if parsed_ok {
        if let Some(app) = doc.get("app").and_then(Value::as_object) {
            app_name = app
                .get("name")
                .and_then(Value::as_str)
                .map(|s| s.to_string());
            app_version = app
                .get("version")
                .and_then(Value::as_str)
                .map(|s| s.to_string());
        }

        if let Some(rows) = doc.get("rows").and_then(Value::as_array) {
            rows_count = rows.len() as u64;
            for r in rows {
                if let Some(scope) = r.get(0).and_then(Value::as_str) {
                    if !scopes.contains(&scope.to_string()) {
                        scopes.push(scope.to_string());
                    }
                }
            }
        }
    }

    let (has_root_help, has_root_version, has_root_cli_specrows) = required_root_rows_present(&doc);

    if parsed_ok && schema_valid {
        if !has_root_help {
            diagnostics.push(Diagnostic::new(
                "X07WASM_SPECROWS_MISSING_ROOT_HELP",
                Severity::Error,
                Stage::Run,
                "missing root help row".to_string(),
            ));
        }
        if !has_root_version {
            diagnostics.push(Diagnostic::new(
                "X07WASM_SPECROWS_MISSING_ROOT_VERSION",
                Severity::Error,
                Stage::Run,
                "missing root version row".to_string(),
            ));
        }
        if !has_root_cli_specrows {
            diagnostics.push(Diagnostic::new(
                "X07WASM_SPECROWS_MISSING_ROOT_CLI_SPECROWS",
                Severity::Error,
                Stage::Run,
                "missing root --cli-specrows row".to_string(),
            ));
        }
    }

    let ordering_checked = parsed_ok && schema_valid;
    let ordering_ok = if ordering_checked {
        match check_canonical_ordering(&doc) {
            Ok(ok) => ok,
            Err(err) => {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_SPECROWS_ORDERING_CHECK_FAILED",
                    Severity::Error,
                    Stage::Run,
                    format!("{err:#}"),
                ));
                false
            }
        }
    } else {
        false
    };
    if ordering_checked && !ordering_ok {
        diagnostics.push(Diagnostic::new(
            "X07WASM_SPECROWS_ORDERING_INVALID",
            Severity::Error,
            Stage::Run,
            "rows are not in canonical ordering".to_string(),
        ));
    }

    if parsed_ok && schema_valid {
        let dups = find_longopt_duplicates(&doc);
        if !dups.is_empty() {
            let mut d = Diagnostic::new(
                "X07WASM_SPECROWS_DUPLICATE_LONGOPT",
                Severity::Error,
                Stage::Run,
                "duplicate --longopt within a scope".to_string(),
            );
            d.data.insert("duplicates".to_string(), json!(dups));
            diagnostics.push(d);
        }
    }

    if parsed_ok && schema_valid && app_name.as_deref() != Some(args.expect_app_name.as_str()) {
        diagnostics.push(Diagnostic::new(
            "X07WASM_SPECROWS_APP_NAME_MISMATCH",
            Severity::Error,
            Stage::Run,
            format!(
                "unexpected app.name: got={:?} expect={:?}",
                app_name, args.expect_app_name
            ),
        ));
    }

    let ok = diagnostics.iter().all(|d| d.severity != Severity::Error);
    let exit_code = report::exit_code::exit_code_for_diagnostics(&diagnostics);

    let report_doc = json!({
        "schema_version": "x07.wasm.cli.specrows.check.report@0.1.0",
        "command": "x07-wasm.cli.specrows.check",
        "ok": ok,
        "exit_code": exit_code,
        "diagnostics": diagnostics,
        "meta": meta,
        "result": {
          "mode": mode,
          "input": input_digest,
          "parsed_ok": parsed_ok,
          "schema_id": schema_id,
          "schema_valid": schema_valid,
          "rows_count": rows_count,
          "scopes": scopes,
          "invariants": {
            "expect_app_name": args.expect_app_name,
            "app_name": app_name,
            "app_version": app_version,
            "has_root_help": has_root_help,
            "has_root_version": has_root_version,
            "has_root_cli_specrows": has_root_cli_specrows,
            "ordering_checked": ordering_checked,
            "ordering_ok": ordering_ok,
          }
        }
    });

    store
        .validate_report_and_emit(scope, machine, started, raw_argv, report_doc)
        .context("emit report")?;

    Ok(exit_code)
}

fn required_root_rows_present(doc: &Value) -> (bool, bool, bool) {
    let Some(rows) = doc.get("rows").and_then(Value::as_array) else {
        return (false, false, false);
    };

    let mut has_help = false;
    let mut has_version = false;
    let mut has_cli_specrows = false;

    for r in rows {
        let Some(arr) = r.as_array() else { continue };
        if arr.len() < 2 {
            continue;
        }
        let Some(scope) = arr.first().and_then(Value::as_str) else {
            continue;
        };
        if scope != "root" {
            continue;
        }
        let Some(kind) = arr.get(1).and_then(Value::as_str) else {
            continue;
        };
        match kind {
            "help" => has_help = true,
            "version" => has_version = true,
            "flag" => {
                if arr.get(3).and_then(Value::as_str) == Some("--cli-specrows") {
                    has_cli_specrows = true;
                }
            }
            _ => {}
        }
    }

    (has_help, has_version, has_cli_specrows)
}

fn check_canonical_ordering(doc: &Value) -> Result<bool> {
    let Some(rows) = doc.get("rows").and_then(Value::as_array) else {
        return Ok(false);
    };

    let mut canon = rows.clone();
    canon.sort_by_key(canonical_row_key);
    Ok(&canon == rows)
}

fn canonical_row_key(row: &Value) -> (String, u8, String, String, String) {
    let arr = row.as_array().cloned().unwrap_or_default();
    let scope = arr
        .first()
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let kind = arr.get(1).and_then(Value::as_str).unwrap_or("").to_string();

    let scope_key = if scope == "root" {
        String::new()
    } else {
        scope.clone()
    };

    let kind_ord = match kind.as_str() {
        "about" => 0,
        "help" => 1,
        "version" => 2,
        "flag" => 3,
        "opt" => 4,
        "arg" => 5,
        _ => 9,
    };

    let long_opt = arr.get(3).and_then(Value::as_str).unwrap_or("").to_string();
    let short_opt = arr.get(2).and_then(Value::as_str).unwrap_or("").to_string();
    let key = arr.get(4).and_then(Value::as_str).unwrap_or("").to_string();

    (scope_key, kind_ord, long_opt, short_opt, key)
}

fn find_longopt_duplicates(doc: &Value) -> Vec<(String, String)> {
    let Some(rows) = doc.get("rows").and_then(Value::as_array) else {
        return Vec::new();
    };

    let mut seen = std::collections::BTreeSet::new();
    let mut dups = std::collections::BTreeSet::new();

    for r in rows {
        let Some(arr) = r.as_array() else { continue };
        if arr.len() < 4 {
            continue;
        }
        let Some(scope) = arr.first().and_then(Value::as_str) else {
            continue;
        };
        let Some(kind) = arr.get(1).and_then(Value::as_str) else {
            continue;
        };
        if kind != "flag" && kind != "opt" {
            continue;
        }
        let Some(longopt) = arr.get(3).and_then(Value::as_str) else {
            continue;
        };
        if longopt.is_empty() {
            continue;
        }
        let k = (scope.to_string(), longopt.to_string());
        if !seen.insert(k.clone()) {
            dups.insert(k);
        }
    }

    dups.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn specrows_self_is_schema_valid_and_canonical() {
        let doc = build_specrows_doc();
        let store = SchemaStore::new().unwrap();

        let diags = store
            .validate("https://x07.org/spec/x07cli.specrows.schema.json", &doc)
            .unwrap();
        assert!(diags.is_empty(), "expected schema-valid doc: {diags:?}");

        let (has_help, has_version, has_cli_specrows) = required_root_rows_present(&doc);
        assert!(has_help);
        assert!(has_version);
        assert!(has_cli_specrows);

        assert!(check_canonical_ordering(&doc).unwrap());
        assert!(find_longopt_duplicates(&doc).is_empty());
    }
}
