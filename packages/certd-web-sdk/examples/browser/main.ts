import { CertdWebSdk } from "../../src/index.ts";

type CertificateItem = {
  id: string;
  commonName: string;
  businessNo: string;
  notAfter: string;
};

const gatewayBaseUrl = element<HTMLInputElement>("gatewayBaseUrl");
const agentBaseUrl = element<HTMLInputElement>("agentBaseUrl");
const organizationId = element<HTMLInputElement>("organizationId");
const facilityCode = element<HTMLInputElement>("facilityCode");
const startMonth = element<HTMLInputElement>("startMonth");
const endMonth = element<HTMLInputElement>("endMonth");
const syncMode = element<HTMLSelectElement>("syncMode");
const certificateId = element<HTMLInputElement>("certificateId");
const password = element<HTMLInputElement>("password");
const output = element<HTMLTextAreaElement>("output");
const status = element<HTMLDivElement>("status");
const certList = element<HTMLDivElement>("certList");

element<HTMLButtonElement>("healthButton").addEventListener("click", () => withAction(checkHealth));
element<HTMLButtonElement>("listButton").addEventListener("click", () => withAction(loadCertificates));
element<HTMLButtonElement>("ephemeralButton").addEventListener("click", () => withAction(runEphemeral));
element<HTMLButtonElement>("storedButton").addEventListener("click", () => withAction(runStored));

setStatus("대기 중");
appendLog("예시 페이지 준비 완료");

async function checkHealth() {
  const response = await fetch(`${agentBaseUrl.value}/healthz`);
  if (!response.ok) {
    throw new Error(`agent health check failed with status ${response.status}`);
  }
  appendLog("agent /healthz -> ok");
  setStatus("Agent 정상");
}

async function loadCertificates() {
  const response = await fetch(`${agentBaseUrl.value}/certificates`);
  if (!response.ok) {
    throw new Error(`certificate list failed with status ${response.status}`);
  }
  const body = (await response.json()) as { items: CertificateItem[] };
  renderCertificates(body.items);
  if (body.items[0]) {
    certificateId.value = body.items[0].id;
  }
  appendLog(`agent /certificates -> ${body.items.length}개`);
  setStatus(`인증서 ${body.items.length}개 로드`);
}

async function runEphemeral() {
  const sdk = createSdk();
  const run = await sdk.runEphemeralExecution(organizationId.value, {
    certificateId: certificateId.value,
    password: password.value,
    facilityCode: optional(facilityCode.value),
    startMonth: optional(startMonth.value),
    endMonth: optional(endMonth.value),
    mode: syncMode.value as "full" | "delta",
  });
  appendLog(`ephemeral accepted -> ${JSON.stringify(run, null, 2)}`);
  setStatus("Ephemeral 실행 수락");
}

async function runStored() {
  const sdk = createSdk();
  const cert = await sdk.registerStoredCertificate(organizationId.value, {
    certificateId: certificateId.value,
    password: password.value,
    facilityCode: optional(facilityCode.value),
  });
  appendLog(`stored registered -> ${JSON.stringify(cert, null, 2)}`);
  setStatus("Stored 등록 완료");
}

function createSdk() {
  return new CertdWebSdk({
    gatewayBaseUrl: gatewayBaseUrl.value,
    agentBaseUrl: agentBaseUrl.value,
  });
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

function renderCertificates(items: CertificateItem[]) {
  certList.innerHTML = "";
  if (items.length === 0) {
    certList.innerHTML = `<div class="cert-item">인증서를 찾지 못했습니다.</div>`;
    return;
  }

  for (const item of items) {
    const card = document.createElement("button");
    card.type = "button";
    card.className = "cert-item";
    card.innerHTML = `
      <strong>${escapeHtml(item.commonName || "(commonName 없음)")}</strong>
      <div>businessNo: ${escapeHtml(item.businessNo || "-")}</div>
      <div>notAfter: ${escapeHtml(item.notAfter)}</div>
      <div style="margin-top:6px; word-break:break-all; color:#6b7280;">${escapeHtml(item.id)}</div>
    `;
    card.addEventListener("click", () => {
      certificateId.value = item.id;
      appendLog(`certificate selected -> ${item.id}`);
    });
    certList.appendChild(card);
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

function optional(value: string) {
  const trimmed = value.trim();
  return trimmed === "" ? undefined : trimmed;
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
