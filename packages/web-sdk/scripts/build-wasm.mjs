import { copyFileSync, existsSync, mkdirSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const packageDir = resolve(scriptDir, "..");
const workspaceDir = resolve(packageDir, "..", "..");
const desktopAgentDir = join(workspaceDir, "desktop-agent");
const generatedDir = join(packageDir, "src", "generated");
const wasmOutput = join(generatedDir, "npki_signer.wasm");
const goroot = execFileSync("go", ["env", "GOROOT"], {
  cwd: desktopAgentDir,
  encoding: "utf8",
}).trim();

mkdirSync(generatedDir, { recursive: true });

execFileSync("go", ["build", "-o", wasmOutput, "./cmd/npki-wasm"], {
  cwd: desktopAgentDir,
  env: {
    ...process.env,
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
