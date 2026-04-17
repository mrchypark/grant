type BrowserSdk = typeof import("../../src/index.ts");

const sdkModule = browserSdk();
const fileInput = element<HTMLInputElement>("html5Files");
const gatewayBaseUrl = element<HTMLInputElement>("gatewayBaseUrl");
const bearerToken = element<HTMLInputElement>("bearerToken");
const organizationId = element<HTMLInputElement>("organizationId");
const commonName = element<HTMLInputElement>("commonName");
const businessNo = element<HTMLInputElement>("businessNo");
const password = element<HTMLInputElement>("password");
const facilityCode = element<HTMLInputElement>("facilityCode");
const syncMode = element<HTMLSelectElement>("syncMode");
const output = element<HTMLTextAreaElement>("output");
const status = element<HTMLDivElement>("status");
const selectionResult = element<HTMLDivElement>("selectionResult");
const modeInputs = Array.from(document.querySelectorAll<HTMLInputElement>('input[name="pickerMode"]'));

element<HTMLButtonElement>("inspectButton").addEventListener("click", () => withAction(inspectSelection));
element<HTMLButtonElement>("prepareButton").addEventListener("click", () => withAction(prepareBundle));
element<HTMLButtonElement>("ephemeralButton").addEventListener("click", () => withAction(runEphemeral));
element<HTMLButtonElement>("storedButton").addEventListener("click", () => withAction(registerStored));

for (const input of modeInputs) {
  input.addEventListener("change", applyPickerMode);
}

applyPickerMode();
appendLog("HTML5 browser signing example ready");

function browserSdk(): BrowserSdk {
  const globalSdk = window.CertdWebSdkBrowser;
  if (!globalSdk) {
    throw new Error("CertdWebSdkBrowser global is not available");
  }
  return globalSdk;
}

function applyPickerMode() {
  const mode = activePickerMode();
  sdkModule.configureHtml5CertificateInput(fileInput, { mode });
  selectionResult.innerHTML = "";
  appendLog(`picker mode -> ${mode}`);
}

async function inspectSelection() {
  const selection = sdkModule.findHtml5CertificateFiles(selectedFiles());
  renderSelection(selection.fileNames);
  appendLog(`selection ready -> ${JSON.stringify(selection.fileNames, null, 2)}`);
  setStatus("파일 검증 완료");
}

async function prepareBundle() {
  const prepared = await sdkModule.prepareHtml5CertificateBundle(selectedFiles(), {
    businessNo: businessNo.value,
    commonName: commonName.value,
    password: password.value,
  });

  renderSelection(prepared.selection.fileNames);
  appendLog(
    `bundle prepared -> ${JSON.stringify(
      {
        businessNo: prepared.bundle.businessNo || "(server resolve)",
        commonName: prepared.bundle.commonName || "(certificate parse fallback failed)",
        signCertDerBase64: prepared.bundle.signCertDerBase64.length,
        signPriKeyBase64: prepared.bundle.signPriKeyBase64.length,
        kmCertDerBase64: prepared.bundle.kmCertDerBase64.length,
        kmPriKeyBase64: prepared.bundle.kmPriKeyBase64.length,
      },
      null,
      2,
    )}`,
  );
  setStatus("업로드용 번들 준비 완료");
}

async function runEphemeral() {
  const sdk = createSdk();
  const run = await sdk.runEphemeralWithBrowserCertificate(organizationId.value, {
    certificateFiles: selectedFiles(),
    password: password.value,
    businessNo: businessNo.value,
    commonName: commonName.value,
    facilityCode: optional(facilityCode.value),
    mode: syncMode.value === "delta" ? "delta" : "full",
  });
  appendLog(`ephemeral run -> ${JSON.stringify(run, null, 2)}`);
  setStatus(`Ephemeral ${run.status}`);
}

async function registerStored() {
  const sdk = createSdk();
  const certificate = await sdk.registerStoredWithBrowserCertificate(organizationId.value, {
    certificateFiles: selectedFiles(),
    password: password.value,
    businessNo: businessNo.value,
    commonName: commonName.value,
  });
  appendLog(`stored certificate -> ${JSON.stringify(certificate, null, 2)}`);
  setStatus("Stored 등록 완료");
}

function createSdk() {
  return new sdkModule.CertdWebSdk({
    gatewayBaseUrl: gatewayBaseUrl.value,
    agentBaseUrl: "http://127.0.0.1:8765",
    bearerToken: optional(bearerToken.value),
    activeOrganizationId: organizationId.value,
  });
}

function renderSelection(fileNames: Record<string, string>) {
  selectionResult.innerHTML = Object.entries(fileNames)
    .map(
      ([key, value]) => `
        <div class="result-item">
          <strong>${escapeHtml(key)}</strong>
          <div>${escapeHtml(value)}</div>
        </div>
      `,
    )
    .join("");
}

function selectedFiles(): File[] {
  const files = fileInput.files ? Array.from(fileInput.files) : [];
  if (files.length === 0) {
    throw new Error("certificate files are required");
  }
  return files;
}

function activePickerMode() {
  return modeInputs.find((item) => item.checked)?.value === "files" ? "files" : "directory";
}

function optional(value: string) {
  const trimmed = value.trim();
  return trimmed === "" ? undefined : trimmed;
}

async function withAction(action: () => Promise<void>) {
  try {
    setStatus("실행 중");
    await action();
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    appendLog(`error -> ${message}`);
    setStatus(message, true);
  }
}

function setStatus(message: string, isError = false) {
  status.textContent = message;
  status.className = isError ? "status error" : "status";
}

function appendLog(message: string) {
  const timestamp = new Date().toLocaleTimeString("ko-KR", { hour12: false });
  output.value = `[${timestamp}] ${message}\n${output.value}`;
}

function element<T extends HTMLElement>(id: string): T {
  const node = document.getElementById(id);
  if (!node) {
    throw new Error(`missing element: ${id}`);
  }
  return node as T;
}

function escapeHtml(value: string) {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}
