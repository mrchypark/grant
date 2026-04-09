import type { AgentProofBundle } from "./agent.ts";

export type ChallengeMode = "stored" | "ephemeral";
export type SyncMode = "full" | "delta";

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

export class GatewayClient {
  constructor(private readonly baseUrl: string) {}

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

  private async post<T>(path: string, body: unknown): Promise<T> {
    const response = await fetch(`${this.baseUrl}${path}`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
    });
    if (!response.ok) {
      throw new Error(`gateway request failed with status ${response.status}`);
    }
    return (await response.json()) as T;
  }
}
