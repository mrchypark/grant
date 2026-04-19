import { copyFileSync, existsSync, mkdirSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const goBin = process.env.GO ?? "go";
const goToolchain = process.env.GOTOOLCHAIN ?? "auto";
// Keep this version in sync with desktop-agent/Makefile.
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
const goroot = execFileSync(goBin, ["env", "GOROOT"], {
  cwd: desktopAgentDir,
  env: { ...process.env, GOTOOLCHAIN: goToolchain },
  encoding: "utf8",
}).trim();
const goToolPath = `${join(goroot, "bin")}${process.platform === "win32" ? ";" : ":"}${process.env.PATH ?? ""}`;

mkdirSync(generatedDir, { recursive: true });

let buildCommand = goBin;
let buildArgs = ["build"];

if (garbleEnabled) {
  const [hostGoos, hostGoarch] = execFileSync(goBin, ["env", "GOHOSTOS", "GOHOSTARCH"], {
    cwd: desktopAgentDir,
    env: { ...process.env, GOTOOLCHAIN: goToolchain },
    encoding: "utf8",
  })
    .trim()
    .split(/\s+/);
  const garbleBin = join(
    desktopAgentDir,
    ".bin",
    `${hostGoos}-${hostGoarch}`,
    garbleVersion,
    hostGoos === "windows" ? "garble.exe" : "garble",
  );

  if (!existsSync(garbleBin)) {
    mkdirSync(dirname(garbleBin), { recursive: true });
    execFileSync(goBin, ["install", `mvdan.cc/garble@${garbleVersion}`], {
      cwd: desktopAgentDir,
      env: {
        ...process.env,
        GOBIN: dirname(garbleBin),
        GOTOOLCHAIN: goToolchain,
        GOOS: "",
        GOARCH: "",
      },
      stdio: "inherit",
    });
  }

  buildCommand = garbleBin;
  buildArgs = [...garbleFlags, "build"];
}

execFileSync(buildCommand, [...buildArgs, "-o", wasmOutput, "./cmd/npki-wasm"], {
  cwd: desktopAgentDir,
  env: {
    ...process.env,
    GOTOOLCHAIN: goToolchain,
    PATH: goToolPath,
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
