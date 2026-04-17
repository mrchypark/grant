import {
  DesktopAgentClient,
  type AgentApprovalRequest,
} from "./agent.ts";
import {
  GatewayClient,
  type Challenge,
  type CreateChallengeInput,
  type ProofBundle,
  type StoredCertificate,
  type SyncMode,
  type SyncRun,
} from "./client.ts";
import {
  configureHtml5CertificateInput,
  findHtml5CertificateFiles,
  prepareHtml5CertificateBundle,
  type Html5CertificateBundleInput,
  type Html5CertificateFileKey,
  type Html5CertificateInputMode,
  type Html5CertificateSelection,
  type PreparedHtml5CertificateBundle,
} from "./html5.ts";
import {
  prepareHtml5ChallengeProof,
  signChallengeWithProofBundle,
  type PreparedHtml5ChallengeProof,
} from "./npki-browser.ts";
import { configureNpkiWasmAssets } from "./npki-wasm.ts";

export type CertdWebSdkOptions = {
  gatewayBaseUrl: string;
  agentBaseUrl: string;
  bearerToken?: string;
  activeOrganizationId?: string;
};

export type SignAndRunInput = {
  businessNo?: string;
  facilityCode?: string;
  startMonth?: string;
  endMonth?: string;
  mode?: SyncMode;
};

export type RegisterStoredInput = {
  businessNo?: string;
};

export type RunStoredInput = {
  certificateId: string;
};

export type BrowserCertificateInput = {
  certificateFiles: Iterable<File>;
  password: string;
  businessNo?: string;
  commonName?: string;
  challengeTtlSeconds?: number;
};

export type BrowserEphemeralRunInput = BrowserCertificateInput & {
  facilityCode?: string;
  startMonth?: string;
  endMonth?: string;
  mode?: SyncMode;
};

export type AgentCapabilities = {
  available: true;
  flows: {
    ephemeral: true;
    stored: true;
  };
};

export class CertdWebSdk {
  readonly raw: {
    gateway: GatewayClient;
    agent: DesktopAgentClient;
  };

  constructor(options: CertdWebSdkOptions) {
    const gateway = new GatewayClient({
      baseUrl: trimTrailingSlash(options.gatewayBaseUrl),
      bearerToken: options.bearerToken,
      activeOrganizationId: options.activeOrganizationId,
    });
    const agent = new DesktopAgentClient(trimTrailingSlash(options.agentBaseUrl));
    this.raw = { gateway, agent };
  }

  async createChallenge(
    organizationId: string,
    input: CreateChallengeInput,
  ): Promise<Challenge> {
    return this.raw.gateway.createChallenge(organizationId, input);
  }

  async assertAvailable(): Promise<void> {
    await this.raw.agent.assertHealthy();
  }

  async connect(): Promise<AgentCapabilities> {
    return this.getCapabilities();
  }

  async getCapabilities(): Promise<AgentCapabilities> {
    await this.assertAvailable();
    return {
      available: true,
      flows: {
        ephemeral: true,
        stored: true,
      },
    };
  }

  async runEphemeral(
    organizationId: string,
    input: SignAndRunInput,
  ): Promise<SyncRun> {
    await this.assertAvailable();
    const result = await this.raw.agent.approveEphemeralRun(
      approvalRequestFrom(this.raw.gateway, organizationId, input),
    );
    return result as unknown as SyncRun;
  }

  async registerStored(
    organizationId: string,
    input: RegisterStoredInput,
  ): Promise<StoredCertificate> {
    await this.assertAvailable();
    const result = await this.raw.agent.approveStoredRegistration(
      approvalRequestFrom(this.raw.gateway, organizationId, input),
    );
    return result as unknown as StoredCertificate;
  }

  async listStoredCertificates(organizationId: string): Promise<StoredCertificate[]> {
    return this.raw.gateway.listStoredCertificates(organizationId);
  }

  async runStored(
    organizationId: string,
    input: RunStoredInput,
  ): Promise<SyncRun> {
    const issued = await this.raw.gateway.issueCrawlTaskToken(
      organizationId,
      input.certificateId,
    );
    return this.raw.gateway.startCrawl(organizationId, issued.crawlTaskToken);
  }

  async runEphemeralExecution(organizationId: string, input: SignAndRunInput): Promise<SyncRun> {
    return this.runEphemeral(organizationId, input);
  }

  async registerStoredCertificate(organizationId: string, input: RegisterStoredInput): Promise<StoredCertificate> {
    return this.registerStored(organizationId, input);
  }

  async runEphemeralWithBrowserCertificate(
    organizationId: string,
    input: BrowserEphemeralRunInput,
  ): Promise<SyncRun> {
    const challenge = await this.createChallenge(organizationId, {
      mode: "ephemeral",
      facilityCode: trimOptional(input.facilityCode),
      ttlSeconds: input.challengeTtlSeconds,
    });
    const proof = await prepareHtml5ChallengeProof(challenge.payload, input.certificateFiles, {
      businessNo: input.businessNo,
      commonName: input.commonName,
      password: input.password,
    });
    return this.raw.gateway.executeEphemeralExecution(organizationId, {
      challengeId: challenge.challengeId,
      challengePayload: challenge.payload,
      challengeSignatureBase64: proof.challengeSignatureBase64,
      bundle: proof.bundle,
      facilityCode: trimOptional(input.facilityCode),
      startMonth: trimOptional(input.startMonth),
      endMonth: trimOptional(input.endMonth),
      mode: trimOptional(input.mode),
    });
  }

  async registerStoredWithBrowserCertificate(
    organizationId: string,
    input: BrowserCertificateInput,
  ): Promise<StoredCertificate> {
    const challenge = await this.createChallenge(organizationId, {
      mode: "stored",
      ttlSeconds: input.challengeTtlSeconds,
    });
    const proof = await prepareHtml5ChallengeProof(challenge.payload, input.certificateFiles, {
      businessNo: input.businessNo,
      commonName: input.commonName,
      password: input.password,
    });
    return this.raw.gateway.registerStoredCertificate(organizationId, {
      challengeId: challenge.challengeId,
      challengePayload: challenge.payload,
      challengeSignatureBase64: proof.challengeSignatureBase64,
      bundle: proof.bundle,
    });
  }
}

function approvalRequestFrom(
  gateway: GatewayClient,
  organizationId: string,
  input: SignAndRunInput | RegisterStoredInput,
): AgentApprovalRequest {
  const request: AgentApprovalRequest = {
    gatewayBaseUrl: resolveAgentGatewayBaseUrl(gateway.options.baseUrl),
    organizationId,
    bearerToken: gateway.options.bearerToken,
    activeOrganizationId: gateway.options.activeOrganizationId,
  };
  const businessNo = trimOptional(input.businessNo);
  if (businessNo) {
    request.businessNo = businessNo;
  }
  if ("facilityCode" in input) {
    const facilityCode = trimOptional(input.facilityCode);
    const startMonth = trimOptional(input.startMonth);
    const endMonth = trimOptional(input.endMonth);
    const mode = trimOptional(input.mode);
    if (facilityCode) {
      request.facilityCode = facilityCode;
    }
    if (startMonth) {
      request.startMonth = startMonth;
    }
    if (endMonth) {
      request.endMonth = endMonth;
    }
    if (mode) {
      request.mode = mode;
    }
  }
  return request;
}

function resolveAgentGatewayBaseUrl(baseUrl: string): string {
  if (/^https?:\/\//i.test(baseUrl)) {
    return baseUrl;
  }
  const origin = globalThis.location?.origin;
  if (!origin) {
    return baseUrl;
  }
  return new URL(baseUrl, origin).toString().replace(/\/+$/, "");
}

function trimTrailingSlash(value: string): string {
  return value.replace(/\/+$/, "");
}

function trimOptional(value: string | undefined): string | undefined {
  const trimmed = value?.trim();
  return trimmed ? trimmed : undefined;
}

export type {
  AgentCapabilities,
  BrowserCertificateInput,
  BrowserEphemeralRunInput,
  Challenge,
  CreateChallengeInput,
  ProofBundle,
  StoredCertificate,
  SyncRun,
};

export { DesktopAgentClient, GatewayClient };
export {
  configureNpkiWasmAssets,
  configureHtml5CertificateInput,
  findHtml5CertificateFiles,
  prepareHtml5CertificateBundle,
  prepareHtml5ChallengeProof,
  signChallengeWithProofBundle,
};
export type { AgentApprovalRequest, AgentSession } from "./agent.ts";
export type {
  CrawlTaskToken,
  ChallengeMode,
  ExecuteEphemeralExecutionInput,
  IssueCrawlTaskTokenInput,
  RegisterStoredCertificateInput,
  SyncMode,
} from "./client.ts";
export type {
  Html5CertificateBundleInput,
  Html5CertificateFileKey,
  Html5CertificateInputMode,
  Html5CertificateSelection,
  PreparedHtml5ChallengeProof,
  PreparedHtml5CertificateBundle,
};
