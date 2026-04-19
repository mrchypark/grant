import { copyFileSync, existsSync, mkdirSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const goToolchain = process.env.GOTOOLCHAIN ?? "auto";
const garbleVersion = process.env.GARBLE_VERSION ?? "v0.16.0";
const garbleEnabled =
  process.env.GARBLE_ENABLED !== "0" && !process.argv.includes("--plain");
const garbleFlags = (process.env.GARBLE_FLAGS ?? "-literals")
  .split(/\s+/)
  .filter(Boolean);
const scriptDir = dirname(fileURLToPath(import.meta.url));
const packageDir = resolve(scriptDir, "..");
const workspaceDir = resolve(packageDir, "..", "..");
const desktopAgentDir = join(workspaceDir, "desktop-agent");
const generatedDir = join(packageDir, "src", "generated");
const wasmOutput = join(generatedDir, "npki_signer.wasm");
const hostGoos = execFileSync("go", ["env", "GOHOSTOS"], {
  cwd: desktopAgentDir,
  encoding: "utf8",
}).trim();
const garbleBin = join(
  desktopAgentDir,
  ".bin",
  garbleVersion,
  hostGoos === "windows" ? "garble.exe" : "garble",
);
const goroot = execFileSync("go", ["env", "GOROOT"], {
  cwd: desktopAgentDir,
  encoding: "utf8",
}).trim();

mkdirSync(generatedDir, { recursive: true });

if (garbleEnabled && !existsSync(garbleBin)) {
  mkdirSync(dirname(garbleBin), { recursive: true });
  execFileSync("go", ["install", `mvdan.cc/garble@${garbleVersion}`], {
    cwd: desktopAgentDir,
    env: {
      ...process.env,
      GOBIN: dirname(garbleBin),
      GOTOOLCHAIN: goToolchain,
    },
    stdio: "inherit",
  });
}

const buildCommand = garbleEnabled ? garbleBin : "go";
const buildArgs = garbleEnabled ? [...garbleFlags, "build"] : ["build"];

execFileSync(buildCommand, [...buildArgs, "-o", wasmOutput, "./cmd/npki-wasm"], {
  cwd: desktopAgentDir,
  env: {
    ...process.env,
    GOTOOLCHAIN: goToolchain,
    GOOS: "js",
    GOARCH: "wasm",
  },
  stdio: "inherit",
});

const wasmExecCandidates = [
  join(goroot, "lib", "wasm", "wasm_exec.js"),
  join(goroot, "misc", "wasm", "wasm_exec.js"),
];
const wasmExecSource = wasmExecCandidates.find((candidate) => existsSync(candidate));
if (!wasmExecSource) {
  throw new Error("could not locate wasm_exec.js in GOROOT");
}

copyFileSync(wasmExecSource, join(generatedDir, "wasm_exec.js"));
