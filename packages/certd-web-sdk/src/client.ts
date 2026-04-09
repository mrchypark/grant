import type { AgentProofBundle } from "./agent.ts";

export type ChallengeMode = "stored" | "ephemeral";
export type SyncMode = "full" | "delta";

export type GatewayClientOptions = {
  baseUrl: string;
  bearerToken?: string;
  activeOrganizationId?: string;
};

export type Challenge = {
  challengeId: string;
  mode: ChallengeMode;
  nonce: string;
  payload: string;
  expiresAt: string;
};

export type CreateChallengeInput = {
  mode: ChallengeMode;
  facilityCode?: string;
  ttlSeconds?: number;
};

export type ProofBundle = AgentProofBundle & {
  password: string;
};

export type SyncRun = {
  id: string;
  organizationId: string;
  plantId: string | null;
  mode: string;
  status: string;
  startedAt: string | null;
  finishedAt: string | null;
  durationMs: number;
};

export type StoredCertificate = {
  id: string;
  organizationId: string;
  businessNo: string | null;
  commonName: string;
};

export type CrawlTaskToken = {
  crawlTaskToken: string;
  expiresAt: string;
};

export type ExecuteEphemeralExecutionInput = {
  challengeId: string;
  challengePayload: string;
  bundle: ProofBundle;
  facilityCode?: string;
  startMonth?: string;
  endMonth?: string;
  mode?: SyncMode;
};

export type RegisterStoredCertificateInput = {
  challengeId: string;
  challengePayload: string;
  bundle: ProofBundle;
};

export type IssueCrawlTaskTokenInput = {
  facilityCode?: string;
  startMonth?: string;
  endMonth?: string;
  mode?: SyncMode;
  ttlSeconds?: number;
};

export class GatewayClient {
  constructor(readonly options: GatewayClientOptions) {}

  async createChallenge(organizationId: string, input: CreateChallengeInput): Promise<Challenge> {
    return this.post<Challenge>(
      `/api/v1/organizations/${organizationId}/cert-gateway/challenges`,
      input,
    );
  }

  async executeEphemeralExecution(
    organizationId: string,
    input: ExecuteEphemeralExecutionInput,
  ): Promise<SyncRun> {
    return this.post<SyncRun>(
      `/api/v1/organizations/${organizationId}/cert-gateway/ephemeral-executions`,
      input,
    );
  }

  async registerStoredCertificate(
    organizationId: string,
    input: RegisterStoredCertificateInput,
  ): Promise<StoredCertificate> {
    return this.post<StoredCertificate>(`/api/v1/organizations/${organizationId}/certificates`, input);
  }

  async listStoredCertificates(organizationId: string): Promise<StoredCertificate[]> {
    return this.get<StoredCertificate[]>(`/api/v1/organizations/${organizationId}/certificates`);
  }

  async issueCrawlTaskToken(
    organizationId: string,
    certificateId: string,
    input: IssueCrawlTaskTokenInput = {},
  ): Promise<CrawlTaskToken> {
    return this.post<CrawlTaskToken>(
      `/api/v1/organizations/${organizationId}/certificates/${certificateId}/crawl-tokens`,
      input,
    );
  }

  async startCrawl(organizationId: string, crawlTaskToken: string): Promise<SyncRun> {
    const response = await fetch(`${this.baseUrl}/api/v1/organizations/${organizationId}/crawls`, {
      method: "POST",
      headers: {
        authorization: `Bearer ${crawlTaskToken}`,
      },
    });
    if (!response.ok) {
      throw new Error(`gateway request failed with status ${response.status}`);
    }
    return (await response.json()) as SyncRun;
  }

  private async post<T>(path: string, body: unknown): Promise<T> {
    const response = await fetch(`${this.baseUrl}${path}`, {
      method: "POST",
      headers: this.headers(),
      body: JSON.stringify(body),
    });
    if (!response.ok) {
      throw new Error(`gateway request failed with status ${response.status}`);
    }
    return (await response.json()) as T;
  }

  private async get<T>(path: string): Promise<T> {
    const response = await fetch(`${this.baseUrl}${path}`, {
      headers: this.headers(),
    });
    if (!response.ok) {
      throw new Error(`gateway request failed with status ${response.status}`);
    }
    return (await response.json()) as T;
  }

  private get baseUrl(): string {
    return this.options.baseUrl;
  }

  private headers(): HeadersInit {
    const headers: Record<string, string> = {
      "content-type": "application/json",
    };
    if (this.options.bearerToken) {
      headers.authorization = `Bearer ${this.options.bearerToken}`;
    }
    if (this.options.activeOrganizationId) {
      headers["x-organization-id"] = this.options.activeOrganizationId;
    }
    return headers;
  }
}
