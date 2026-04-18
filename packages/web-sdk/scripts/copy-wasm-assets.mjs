import { copyFileSync, mkdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const packageDir = resolve(scriptDir, "..");
const generatedDir = join(packageDir, "src", "generated");
const targetDir = resolve(packageDir, process.argv[2] ?? "dist/browser");

mkdirSync(targetDir, { recursive: true });
copyFileSync(join(generatedDir, "npki_signer.wasm"), join(targetDir, "npki_signer.wasm"));
copyFileSync(join(generatedDir, "wasm_exec.js"), join(targetDir, "wasm_exec.js"));
