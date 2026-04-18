import type { EscrowProofBundle } from "./client.ts";
import { prepareBundleWithWasm } from "./npki-wasm.ts";

const REQUIRED_CERTIFICATE_FILES = {
  signCertDer: "signCert.der",
  signPriKey: "signPri.key",
  kmCertDer: "kmCert.der",
  kmPriKey: "kmPri.key",
} as const;

export type Html5CertificateFileKey = keyof typeof REQUIRED_CERTIFICATE_FILES;

export type Html5CertificateInputMode = "files" | "directory";

export type Html5CertificateSelection = {
  files: Record<Html5CertificateFileKey, File>;
  fileNames: Record<Html5CertificateFileKey, string>;
};

export type Html5CertificateBundleInput = {
  businessNo?: string;
  commonName?: string;
  password: string;
};

export type PreparedHtml5CertificateBundle = {
  bundle: EscrowProofBundle;
  selection: Html5CertificateSelection;
};

export function configureHtml5CertificateInput(
  input: Html5InputLike,
  options: { mode?: Html5CertificateInputMode } = {},
): Html5InputLike {
  input.type = "file";
  input.multiple = true;
  input.accept = ".der,.key,application/octet-stream";
  if ((options.mode ?? "files") === "directory") {
    input.setAttribute("webkitdirectory", "");
    input.setAttribute("directory", "");
    return input;
  }
  input.removeAttribute("webkitdirectory");
  input.removeAttribute("directory");
  return input;
}

export function findHtml5CertificateFiles(files: Iterable<File>): Html5CertificateSelection {
  const buckets = new Map<string, Map<string, File>>();

  for (const file of files) {
    const fileName = sourceFileName(file);
    if (!isRequiredCertificateBasename(fileName)) {
      continue;
    }

    const bucketKey = bundleDirectoryKey(file);
    const bucket = buckets.get(bucketKey) ?? new Map<string, File>();
    if (bucket.has(fileName)) {
      throw new Error(`duplicate certificate file selected: ${displayFileName(file)}`);
    }
    bucket.set(fileName, file);
    buckets.set(bucketKey, bucket);
  }

  const completeBucket = Array.from(buckets.entries())
    .filter(([, bucket]) => hasCompleteBundle(bucket))
    .sort(([left], [right]) => left.localeCompare(right));

  if (completeBucket.length === 0) {
    const missing = html5CertificateFileKeys()
      .map((key) => REQUIRED_CERTIFICATE_FILES[key])
      .find((requiredFileName) => !Array.from(buckets.values()).some((bucket) => bucket.has(requiredFileName)));
    throw new Error(`missing certificate file: ${missing ?? "signCert.der"}`);
  }

  if (completeBucket.length > 1) {
    const candidates = completeBucket
      .map(([key]) => key || "(current directory)")
      .join(", ");
    throw new Error(`multiple certificate bundles selected; choose one certificate directory: ${candidates}`);
  }

  return selectionFromBucket(completeBucket[0][1]);
}

export async function prepareHtml5CertificateBundle(
  files: Iterable<File>,
  input: Html5CertificateBundleInput,
): Promise<PreparedHtml5CertificateBundle> {
  const selection = findHtml5CertificateFiles(files);
  const password = trimRequired(input.password, "password");
  const bytes = await readHtml5CertificateBytes(selection);
  const bundle = await prepareBundleWithWasm({
    ...bytes,
    businessNo: trimOptional(input.businessNo),
    commonName: trimOptional(input.commonName),
  });

  return {
    selection,
    bundle: { ...bundle, password },
  };
}

export async function readHtml5CertificateBytes(selection: Html5CertificateSelection) {
  const [
    signCertDerBytes,
    signPriKeyBytes,
    kmCertDerBytes,
    kmPriKeyBytes,
  ] = await Promise.all([
    selection.files.signCertDer.arrayBuffer(),
    selection.files.signPriKey.arrayBuffer(),
    selection.files.kmCertDer.arrayBuffer(),
    selection.files.kmPriKey.arrayBuffer(),
  ]);

  return {
    signCertDer: new Uint8Array(signCertDerBytes),
    signPriKey: new Uint8Array(signPriKeyBytes),
    kmCertDer: new Uint8Array(kmCertDerBytes),
    kmPriKey: new Uint8Array(kmPriKeyBytes),
  };
}

function html5CertificateFileKeys(): Html5CertificateFileKey[] {
  return Object.keys(REQUIRED_CERTIFICATE_FILES) as Html5CertificateFileKey[];
}

function selectionFromBucket(bucket: Map<string, File>): Html5CertificateSelection {
  const matched = {} as Record<Html5CertificateFileKey, File>;
  const fileNames = {} as Record<Html5CertificateFileKey, string>;

  for (const key of html5CertificateFileKeys()) {
    const requiredFileName = REQUIRED_CERTIFICATE_FILES[key];
    const file = bucket.get(requiredFileName);
    if (!file) {
      throw new Error(`missing certificate file: ${requiredFileName}`);
    }
    matched[key] = file;
    fileNames[key] = displayFileName(file);
  }

  return { files: matched, fileNames };
}

function hasCompleteBundle(bucket: Map<string, File>): boolean {
  return html5CertificateFileKeys().every((key) => bucket.has(REQUIRED_CERTIFICATE_FILES[key]));
}

function isRequiredCertificateBasename(fileName: string): boolean {
  return html5CertificateFileKeys().some((key) => REQUIRED_CERTIFICATE_FILES[key] === fileName);
}

function bundleDirectoryKey(file: File): string {
  const fullName = displayFileName(file);
  const lastSlash = Math.max(fullName.lastIndexOf("/"), fullName.lastIndexOf("\\"));
  if (lastSlash < 0) {
    return "";
  }
  return fullName.slice(0, lastSlash);
}

function trimRequired(value: string, fieldName: string): string {
  const trimmed = value.trim();
  if (trimmed === "") {
    throw new Error(`${fieldName} is required`);
  }
  return trimmed;
}

function trimOptional(value: string | undefined): string | undefined {
  const trimmed = value?.trim();
  return trimmed ? trimmed : undefined;
}

function sourceFileName(file: File): string {
  return displayFileName(file).split(/[\\/]/).pop() ?? file.name;
}

function displayFileName(file: File): string {
  return readRelativePath(file) || file.name;
}

function readRelativePath(file: File): string {
  const value = (file as File & { webkitRelativePath?: string }).webkitRelativePath;
  return typeof value === "string" ? value : "";
}

type Html5InputLike = Pick<
  HTMLInputElement,
  "accept" | "multiple" | "removeAttribute" | "setAttribute" | "type"
>;
