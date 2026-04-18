import type { ProofBundle } from "./client.ts";

const DEFAULT_WASM_FILE = "npki_signer.wasm";
const DEFAULT_WASM_EXEC_FILE = "wasm_exec.js";
const READY_POLL_INTERVAL_MS = 10;
const READY_TIMEOUT_MS = 5_000;
const EXPORT_NAME_KEY = "__CERTD_WEB_SDK_NPKI_WASM_EXPORT_NAME__";

type WasmEnvelope = {
  ok?: boolean;
  result?: string;
  error?: string;
};

type NpkiWasmApi = {
  inspectCertificate(signCertDer: Uint8Array): WasmEnvelope;
  prepareBundle(
    signCertDer: Uint8Array,
    signPriKey: Uint8Array,
    kmCertDer: Uint8Array,
    kmPriKey: Uint8Array,
    businessNo: string,
    commonName: string,
  ): WasmEnvelope;
  prepareChallengeProof(
    signCertDer: Uint8Array,
    signPriKey: Uint8Array,
    kmCertDer: Uint8Array,
    kmPriKey: Uint8Array,
    businessNo: string,
    commonName: string,
    password: Uint8Array,
    challengePayload: string,
  ): WasmEnvelope;
  signChallengeOnly(
    signPriKey: Uint8Array,
    password: Uint8Array,
    challengePayload: string,
  ): WasmEnvelope;
};

type GoRuntime = {
  importObject: WebAssembly.Imports;
  run(instance: WebAssembly.Instance): Promise<void>;
};

type NpkiWasmAssetUrls = {
  wasmUrl?: string | URL;
  wasmExecUrl?: string | URL;
};

export type WasmCertificateMetadata = {
  commonName: string;
  businessNo: string;
};

export type WasmChallengeProof = {
  bundle: ProofBundle;
  challengeSignatureBase64: string;
};

declare global {
  var Go: undefined | {
    new (): GoRuntime;
  };
}

let assetUrls: NpkiWasmAssetUrls = {};
let runtimeReady: Promise<void> | null = null;
let apiReady: Promise<NpkiWasmApi> | null = null;
let wasmRunPromise: Promise<void> | null = null;
let wasmApi: NpkiWasmApi | null = null;
const runtimeScriptLoads = new Map<string, Promise<void>>();

export function configureNpkiWasmAssets(urls: NpkiWasmAssetUrls): void {
  assetUrls = {
    wasmUrl: urls.wasmUrl ?? assetUrls.wasmUrl,
    wasmExecUrl: urls.wasmExecUrl ?? assetUrls.wasmExecUrl,
  };
  runtimeReady = null;
  apiReady = null;
  wasmRunPromise = null;
  wasmApi = null;
}

export async function inspectCertificateWithWasm(
  signCertDer: Uint8Array,
): Promise<WasmCertificateMetadata> {
  return parseEnvelope<WasmCertificateMetadata>(
    (await ensureNpkiWasmApi()).inspectCertificate(signCertDer),
  );
}

export async function prepareBundleWithWasm(input: {
  signCertDer: Uint8Array;
  signPriKey: Uint8Array;
  kmCertDer: Uint8Array;
  kmPriKey: Uint8Array;
  businessNo?: string;
  commonName?: string;
}): Promise<ProofBundle> {
  return parseEnvelope<ProofBundle>(
    (await ensureNpkiWasmApi()).prepareBundle(
      input.signCertDer,
      input.signPriKey,
      input.kmCertDer,
      input.kmPriKey,
      trimOptional(input.businessNo),
      trimOptional(input.commonName),
    ),
  );
}

export async function prepareChallengeProofWithWasm(input: {
  signCertDer: Uint8Array;
  signPriKey: Uint8Array;
  kmCertDer: Uint8Array;
  kmPriKey: Uint8Array;
  businessNo?: string;
  commonName?: string;
  password: string;
  challengePayload: string;
}): Promise<WasmChallengeProof> {
  const passwordBytes = encodePassword(input.password);
  try {
    return parseEnvelope<WasmChallengeProof>(
      (await ensureNpkiWasmApi()).prepareChallengeProof(
        input.signCertDer,
        input.signPriKey,
        input.kmCertDer,
        input.kmPriKey,
        trimOptional(input.businessNo),
        trimOptional(input.commonName),
        passwordBytes,
        input.challengePayload,
      ),
    );
  } finally {
    passwordBytes.fill(0);
  }
}

export async function signChallengeWithWasm(
  signPriKey: Uint8Array,
  password: string,
  challengePayload: string,
): Promise<string> {
  const passwordBytes = encodePassword(password);
  try {
    const result = parseEnvelope<{ challengeSignatureBase64: string }>(
      (await ensureNpkiWasmApi()).signChallengeOnly(
        signPriKey,
        passwordBytes,
        challengePayload,
      ),
    );
    return result.challengeSignatureBase64;
  } finally {
    passwordBytes.fill(0);
  }
}

function parseEnvelope<T>(response: WasmEnvelope): T {
  if (!response?.ok || typeof response.result !== "string") {
    throw new Error(response?.error ?? "failed to invoke NPKI wasm runtime");
  }
  return JSON.parse(response.result) as T;
}

async function ensureNpkiWasmApi(): Promise<NpkiWasmApi> {
  if (wasmApi) {
    return wasmApi;
  }
  if (apiReady) {
    return apiReady;
  }

  apiReady = (async () => {
    await ensureGoRuntime();
    const exportName = createWasmExportName();
    (globalThis as Record<string, unknown>)[EXPORT_NAME_KEY] = exportName;

    try {
      const wasmBinary = await readBinary(resolveAssetUrl("wasmUrl", DEFAULT_WASM_FILE));
      const go = new globalThis.Go!();
      const result = await WebAssembly.instantiate(wasmBinary, go.importObject);
      wasmRunPromise = go.run(result.instance);
      wasmApi = await waitForApi(exportName);
      return wasmApi;
    } finally {
      delete (globalThis as Record<string, unknown>)[EXPORT_NAME_KEY];
      delete (globalThis as Record<string, unknown>)[exportName];
    }
  })().catch((error) => {
    apiReady = null;
    wasmRunPromise = null;
    wasmApi = null;
    throw error;
  });

  return apiReady;
}

async function ensureGoRuntime(): Promise<void> {
  if (globalThis.Go) {
    return;
  }
  if (runtimeReady) {
    return runtimeReady;
  }

  runtimeReady = (async () => {
    const runtimeUrl = resolveAssetUrl("wasmExecUrl", DEFAULT_WASM_EXEC_FILE);
    await loadGoRuntimeScript(runtimeUrl);
    if (!globalThis.Go) {
      throw new Error("failed to initialize Go wasm runtime");
    }
  })().catch((error) => {
    runtimeReady = null;
    throw error;
  });

  return runtimeReady;
}

async function waitForApi(exportName: string): Promise<NpkiWasmApi> {
  const startedAt = Date.now();
  while (true) {
    const candidate = (globalThis as Record<string, unknown>)[exportName];
    if (candidate && typeof candidate === "object") {
      return candidate as NpkiWasmApi;
    }
    if (Date.now() - startedAt > READY_TIMEOUT_MS) {
      throw new Error("timed out waiting for NPKI wasm runtime");
    }
    await new Promise((resolve) => setTimeout(resolve, READY_POLL_INTERVAL_MS));
  }
}

function resolveAssetUrl(key: keyof NpkiWasmAssetUrls, fileName: string): URL {
  const configured = assetUrls[key];
  const url = configured
    ? configured instanceof URL
      ? configured
      : new URL(configured, globalThis.location?.href ?? "file:///")
    : new URL(`./generated/${fileName}`, import.meta.url);

  assertSecureAssetUrl(url, key);
  return url;
}

async function readBinary(url: URL): Promise<ArrayBuffer> {
  if (url.protocol === "file:") {
    const { readFile } = await loadNodeFsPromises();
    const buffer = await readFile(url);
    return buffer.buffer.slice(buffer.byteOffset, buffer.byteOffset + buffer.byteLength);
  }

  const response = await fetch(url);
  if (!response.ok) {
    throw new Error(`failed to load ${url.toString()}: ${response.status}`);
  }
  return response.arrayBuffer();
}

async function readText(url: URL): Promise<string> {
  if (url.protocol === "file:") {
    const { readFile } = await loadNodeFsPromises();
    return readFile(url, "utf8");
  }

  const response = await fetch(url);
  if (!response.ok) {
    throw new Error(`failed to load ${url.toString()}: ${response.status}`);
  }
  return response.text();
}

function assertSecureAssetUrl(url: URL, key: keyof NpkiWasmAssetUrls): void {
  if (url.protocol === "file:") {
    return;
  }
  if (url.protocol === "https:") {
    if (typeof window !== "undefined" && url.origin !== window.location.origin) {
      throw new Error(`${key} must be same-origin: ${url.toString()}`);
    }
    return;
  }
  if (url.protocol === "http:" && isLoopbackHost(url.hostname)) {
    return;
  }
  throw new Error(`${key} must use https or loopback http: ${url.toString()}`);
}

function ensureTrailingSlash(value: string): string {
  return value.endsWith("/") ? value : `${value}/`;
}

function isLoopbackHost(hostname: string): boolean {
  const normalized = hostname.toLowerCase();
  return normalized === "localhost" || normalized === "127.0.0.1" || normalized === "::1";
}

function encodePassword(password: string): Uint8Array {
  return new TextEncoder().encode(password);
}

function trimOptional(value: string | undefined): string {
  return value?.trim() ?? "";
}

function createWasmExportName(): string {
  if (globalThis.crypto?.randomUUID) {
    return `__certdNpkiWasm_${globalThis.crypto.randomUUID()}`;
  }
  return `__certdNpkiWasm_${Date.now().toString(36)}_${Math.random().toString(36).slice(2)}`;
}

async function loadGoRuntimeScript(url: URL): Promise<void> {
  if (typeof document !== "undefined") {
    await loadBrowserScript(url);
    return;
  }

  const source = await readText(url);
  const { Script } = await loadNodeVm();
  new Script(source, { filename: url.toString() }).runInThisContext();
}

function loadBrowserScript(url: URL): Promise<void> {
  const key = url.toString();
  const existing = runtimeScriptLoads.get(key);
  if (existing) {
    return existing;
  }

  const promise = new Promise<void>((resolve, reject) => {
    const found = document.querySelector<HTMLScriptElement>(`script[data-certd-wasm-runtime="${key}"]`);
    if (found) {
      if (globalThis.Go) {
        resolve();
        return;
      }
      found.addEventListener("load", () => resolve(), { once: true });
      found.addEventListener("error", () => reject(new Error(`failed to load ${key}`)), { once: true });
      return;
    }

    const script = document.createElement("script");
    script.async = true;
    script.dataset.certdWasmRuntime = key;
    script.src = key;
    script.addEventListener("load", () => resolve(), { once: true });
    script.addEventListener("error", () => reject(new Error(`failed to load ${key}`)), { once: true });
    document.head.append(script);
  }).catch((error) => {
    runtimeScriptLoads.delete(key);
    document
      .querySelectorAll<HTMLScriptElement>(`script[data-certd-wasm-runtime="${key}"]`)
      .forEach((script) => script.remove());
    throw error;
  });

  runtimeScriptLoads.set(key, promise);
  return promise;
}

async function loadNodeFsPromises() {
  const dynamicImport = new Function("specifier", "return import(specifier);") as (
    specifier: string,
  ) => Promise<typeof import("node:fs/promises")>;
  return dynamicImport("node:fs/promises");
}

async function loadNodeVm() {
  const dynamicImport = new Function("specifier", "return import(specifier);") as (
    specifier: string,
  ) => Promise<typeof import("node:vm")>;
  return dynamicImport("node:vm");
}
