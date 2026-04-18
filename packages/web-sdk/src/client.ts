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

export type BrowserCertificateIdentity = {
  businessNo: string;
  commonName: string;
  signCertDerBase64: string;
};

export type ProofBundle = Omit<AgentProofBundle, "password">;

export type CreateChallengeInput = {
  mode: ChallengeMode;
  facilityCode?: string;
  ttlSeconds?: number;
};

export type EscrowProofBundle = ProofBundle & {
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
  challengeSignatureBase64: string;
  bundle: EscrowProofBundle;
  facilityCode?: string;
  startMonth?: string;
  endMonth?: string;
  mode?: SyncMode;
};

export type RegisterStoredCertificateInput = {
  challengeId: string;
  challengePayload: string;
  challengeSignatureBase64: string;
  bundle: EscrowProofBundle;
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
    const response = await fetch(this.url(`/api/v1/organizations/${organizationId}/crawls`), {
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
    const response = await fetch(this.url(path), {
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
    const response = await fetch(this.url(path), {
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

  private url(path: string): string {
    return resolveSecureGatewayUrl(this.baseUrl, path).toString();
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

function resolveSecureGatewayUrl(baseUrl: string, path: string): URL {
  const base = /^https?:\/\//i.test(baseUrl)
    ? new URL(ensureTrailingSlash(baseUrl))
    : new URL(baseUrl, resolveBrowserOrigin());
  const absolute = new URL(stripLeadingSlash(path), ensureTrailingSlash(base.toString()));
  assertSecureRemoteUrl(absolute, "gateway");
  return absolute;
}

function resolveBrowserOrigin(): string {
  const origin = globalThis.location?.origin;
  if (!origin) {
    throw new Error("relative gatewayBaseUrl requires a browser origin");
  }
  return ensureTrailingSlash(origin);
}

function assertSecureRemoteUrl(url: URL, label: string): void {
  if (url.protocol === "https:") {
    return;
  }
  if (url.protocol === "http:" && isLoopbackHost(url.hostname)) {
    return;
  }
  throw new Error(`${label} URL must use https or loopback http: ${url.toString()}`);
}

function isLoopbackHost(hostname: string): boolean {
  const normalized = hostname.toLowerCase();
  return normalized === "localhost" || normalized === "127.0.0.1" || normalized === "::1";
}

function ensureTrailingSlash(value: string): string {
  return value.endsWith("/") ? value : `${value}/`;
}

function stripLeadingSlash(value: string): string {
  return value.replace(/^\/+/, "");
}
