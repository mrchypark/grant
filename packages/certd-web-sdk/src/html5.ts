import type { ProofBundle } from "./client.ts";
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
  bundle: ProofBundle;
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
  const indexed = new Map<string, File>();

  for (const file of files) {
    const fileName = sourceFileName(file);
    if (indexed.has(fileName)) {
      throw new Error(`duplicate certificate file selected: ${fileName}`);
    }
    indexed.set(fileName, file);
  }

  const matched = {} as Record<Html5CertificateFileKey, File>;
  const fileNames = {} as Record<Html5CertificateFileKey, string>;

  for (const key of html5CertificateFileKeys()) {
    const requiredFileName = REQUIRED_CERTIFICATE_FILES[key];
    const file = indexed.get(requiredFileName);
    if (!file) {
      throw new Error(`missing certificate file: ${requiredFileName}`);
    }
    matched[key] = file;
    fileNames[key] = displayFileName(file);
  }

  return {
    files: matched,
    fileNames,
  };
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
