import {
  DesktopAgentClient,
  type AgentApprovalRequest,
} from "./agent.ts";
import {
  GatewayClient,
  type BrowserCertificateIdentity,
  type Challenge,
  type CreateChallengeInput,
  type ProofBundle,
  type EscrowProofBundle,
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
  prepareHtml5SignedChallengeProof,
  signChallengeWithProofBundle,
  type PreparedHtml5ChallengeProof,
  type PreparedHtml5SignedChallengeProof,
} from "./npki-browser.ts";

export type WebSdkOptions = {
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
  escrow?: boolean;
};

export type BrowserEphemeralRunInput = BrowserCertificateInput & {
  facilityCode?: string;
  startMonth?: string;
  endMonth?: string;
  mode?: SyncMode;
};

export type BrowserEscrowCertificateInput = BrowserCertificateInput & {
  escrow: true;
};

export type BrowserEscrowEphemeralRunInput = BrowserEphemeralRunInput & {
  escrow: true;
};

export type BrowserChallengeProofInput = Omit<BrowserCertificateInput, "escrow"> & {
  mode: "stored" | "ephemeral";
  facilityCode?: string;
};

export type BrowserSignedChallengeProof = {
  challengeId: string;
  challengePayload: string;
  challengeSignatureBase64: string;
  certificate: BrowserCertificateIdentity;
  expiresAt: string;
  mode: "stored" | "ephemeral";
};

export type AgentCapabilities = {
  available: true;
  flows: {
    ephemeral: true;
    stored: true;
  };
};

export class WebSdk {
  readonly raw: {
    gateway: GatewayClient;
    agent: DesktopAgentClient;
  };

  constructor(options: WebSdkOptions) {
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

  /** @deprecated Use runEphemeralWithEscrowedBrowserCertificate(). */
  async runEphemeralWithBrowserCertificate(
    organizationId: string,
    input: BrowserEphemeralRunInput,
  ): Promise<SyncRun> {
    // Deprecated alias kept for compatibility. Prefer the explicit escrow method.
    assertExplicitEscrow(input, "runEphemeralWithEscrowedBrowserCertificate");
    return this.runEphemeralWithEscrowedBrowserCertificate(organizationId, input);
  }

  async runEphemeralWithEscrowedBrowserCertificate(
    organizationId: string,
    input: BrowserEscrowEphemeralRunInput,
  ): Promise<SyncRun> {
    const requestedSyncMode = trimOptional(input.mode) as SyncMode | undefined;
    const challenge = await this.createChallenge(organizationId, {
      mode: "ephemeral",
      syncMode: requestedSyncMode,
      origin: currentOrigin(),
      businessNo: trimOptional(input.businessNo),
      facilityCode: trimOptional(input.facilityCode),
      startMonth: trimOptional(input.startMonth),
      endMonth: trimOptional(input.endMonth),
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
      facilityCode: trimOptional(challenge.facilityCode) ?? trimOptional(input.facilityCode),
      startMonth: trimOptional(challenge.startMonth),
      endMonth: trimOptional(challenge.endMonth),
      mode: (trimOptional(challenge.syncMode) as SyncMode | undefined) ?? requestedSyncMode,
    });
  }

  /** @deprecated Use registerStoredWithEscrowedBrowserCertificate(). */
  async registerStoredWithBrowserCertificate(
    organizationId: string,
    input: BrowserCertificateInput,
  ): Promise<StoredCertificate> {
    // Deprecated alias kept for compatibility. Prefer the explicit escrow method.
    assertExplicitEscrow(input, "registerStoredWithEscrowedBrowserCertificate");
    return this.registerStoredWithEscrowedBrowserCertificate(organizationId, input);
  }

  async registerStoredWithEscrowedBrowserCertificate(
    organizationId: string,
    input: BrowserEscrowCertificateInput,
  ): Promise<StoredCertificate> {
    const challenge = await this.createChallenge(organizationId, {
      mode: "stored",
      origin: currentOrigin(),
      businessNo: trimOptional(input.businessNo),
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

  async createSignedChallengeProof(
    organizationId: string,
    input: BrowserChallengeProofInput,
  ): Promise<BrowserSignedChallengeProof> {
    const challenge = await this.createChallenge(organizationId, {
      mode: input.mode,
      origin: currentOrigin(),
      businessNo: trimOptional(input.businessNo),
      facilityCode: input.mode === "ephemeral" ? trimOptional(input.facilityCode) : undefined,
      ttlSeconds: input.challengeTtlSeconds,
    });
    const proof = await prepareHtml5SignedChallengeProof(challenge.payload, input.certificateFiles, {
      businessNo: input.businessNo,
      commonName: input.commonName,
      password: input.password,
    });
    return {
      challengeId: challenge.challengeId,
      challengePayload: challenge.payload,
      challengeSignatureBase64: proof.challengeSignatureBase64,
      certificate: proof.certificate,
      expiresAt: challenge.expiresAt,
      mode: input.mode,
    };
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
    const mode = trimOptional(input.mode) as SyncMode | undefined;
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

function currentOrigin(): string | undefined {
  const origin = globalThis.location?.origin;
  return typeof origin === "string" && origin ? origin : undefined;
}

function assertExplicitEscrow(
  input: BrowserCertificateInput,
  methodName: string,
): asserts input is BrowserCertificateInput & { escrow: true } {
  if (input.escrow === true) {
    return;
  }
  throw new Error(
    `Escrow upload is an explicit opt-in. Set escrow: true or use ${methodName}().`,
  );
}

export { DesktopAgentClient, GatewayClient };
export {
  configureHtml5CertificateInput,
  findHtml5CertificateFiles,
  prepareHtml5CertificateBundle,
  prepareHtml5ChallengeProof,
  prepareHtml5SignedChallengeProof,
  signChallengeWithProofBundle,
};
export type { AgentApprovalRequest, AgentSession } from "./agent.ts";
export type {
  Challenge,
  CrawlTaskToken,
  ChallengeMode,
  CreateChallengeInput,
  EscrowProofBundle,
  ExecuteEphemeralExecutionInput,
  IssueCrawlTaskTokenInput,
  IngestLocalExecutionResultInput,
  LocalExecutionSnapshot,
  ProofBundle,
  RegisterStoredCertificateInput,
  StoredCertificate,
  SyncMode,
  SyncRun,
} from "./client.ts";
export type {
  Html5CertificateBundleInput,
  Html5CertificateFileKey,
  Html5CertificateInputMode,
  Html5CertificateSelection,
  PreparedHtml5CertificateBundle,
} from "./html5.ts";
export type {
  PreparedHtml5ChallengeProof,
  PreparedHtml5SignedChallengeProof,
} from "./npki-browser.ts";

export { WebSdk as CertdWebSdk };
export type { WebSdkOptions as CertdWebSdkOptions };
