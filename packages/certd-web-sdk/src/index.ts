import {
  DesktopAgentClient,
  type AgentApprovalRequest,
} from "./agent.ts";
import {
  GatewayClient,
  type Challenge,
  type CreateChallengeInput,
  type StoredCertificate,
  type SyncMode,
  type SyncRun,
} from "./client.ts";

export type CertdWebSdkOptions = {
  gatewayBaseUrl: string;
  agentBaseUrl: string;
  bearerToken?: string;
  activeOrganizationId?: string;
};

export type SignAndRunInput = {
  businessNo: string;
  facilityCode?: string;
  startMonth?: string;
  endMonth?: string;
  mode?: SyncMode;
};

export type RegisterStoredInput = {
  businessNo: string;
};

export type RunStoredInput = {
  certificateId: string;
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
}

function approvalRequestFrom(
  gateway: GatewayClient,
  organizationId: string,
  input: SignAndRunInput | RegisterStoredInput,
): AgentApprovalRequest {
  const request: AgentApprovalRequest = {
    gatewayBaseUrl: resolveAgentGatewayBaseUrl(gateway.options.baseUrl),
    organizationId,
    businessNo: trimRequired("businessNo", input.businessNo),
    bearerToken: gateway.options.bearerToken,
    activeOrganizationId: gateway.options.activeOrganizationId,
  };
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

function trimRequired(field: string, value: string): string {
  const trimmed = value.trim();
  if (!trimmed) {
    throw new Error(`${field} is required`);
  }
  return trimmed;
}

function trimOptional(value: string | undefined): string | undefined {
  const trimmed = value?.trim();
  return trimmed ? trimmed : undefined;
}

export type {
  AgentCapabilities,
  Challenge,
  CreateChallengeInput,
  StoredCertificate,
  SyncRun,
};

export { DesktopAgentClient, GatewayClient };
export type { AgentApprovalRequest, AgentSession } from "./agent.ts";
export type {
  CrawlTaskToken,
  ChallengeMode,
  ExecuteEphemeralExecutionInput,
  IssueCrawlTaskTokenInput,
  ProofBundle,
  RegisterStoredCertificateInput,
  SyncMode,
} from "./client.ts";
