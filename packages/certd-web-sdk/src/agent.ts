export type AgentProofBundle = {
  businessNo: string;
  commonName: string;
  signCertDerBase64: string;
  signPriKeyBase64: string;
  kmCertDerBase64: string;
  kmPriKeyBase64: string;
  password?: string;
};

export type AgentSignRequest = {
  certificateId: string;
  challenge: string;
  password: string;
  businessNo?: string;
};

export type AgentSignResponse = {
  certificateId: string;
  challenge: string;
  challengeSignatureBase64: string;
  bundle: AgentProofBundle;
};

export type AgentSession = {
  sessionToken: string;
  expiresAt: string;
};

export type AgentApprovalRequest = {
  gatewayBaseUrl: string;
  organizationId: string;
  businessNo: string;
  bearerToken?: string;
  activeOrganizationId?: string;
};

export class DesktopAgentClient {
  private sessionToken?: string;

  constructor(private readonly baseUrl: string) {}

  async assertHealthy(): Promise<void> {
    const response = await fetch(`${this.baseUrl}/healthz`);
    if (!response.ok) {
      throw new Error(`agent health check failed with status ${response.status}`);
    }
  }

  async openSession(): Promise<AgentSession> {
    const response = await fetch(`${this.baseUrl}/sessions/open`, {
      method: "POST",
    });
    if (!response.ok) {
      throw new Error(`agent session open failed with status ${response.status}`);
    }
    const session = (await response.json()) as AgentSession;
    this.sessionToken = session.sessionToken;
    return session;
  }

  async sign(input: AgentSignRequest): Promise<AgentSignResponse> {
    const response = await this.fetchWithSessionRetry("/sign", {
      method: "POST",
      body: JSON.stringify(input),
    });
    if (!response.ok) {
      throw new Error(`agent sign failed with status ${response.status}`);
    }
    return (await response.json()) as AgentSignResponse;
  }

  async listCertificates(): Promise<{ items: Array<Record<string, unknown>> }> {
    const response = await this.fetchWithSessionRetry("/certificates");
    if (!response.ok) {
      throw new Error(`agent certificate list failed with status ${response.status}`);
    }
    return (await response.json()) as { items: Array<Record<string, unknown>> };
  }

  async approveEphemeralRun(input: AgentApprovalRequest): Promise<Record<string, unknown>> {
    return this.postApproval("/approve-ephemeral-run", input);
  }

  async approveStoredRegistration(input: AgentApprovalRequest): Promise<Record<string, unknown>> {
    return this.postApproval("/approve-stored-registration", input);
  }

  private async ensureSession(): Promise<void> {
    if (this.sessionToken) {
      return;
    }
    await this.openSession();
  }

  private headers(): HeadersInit {
    const headers: Record<string, string> = {
      "content-type": "application/json",
    };
    if (this.sessionToken) {
      headers["x-agent-session"] = this.sessionToken;
    }
    return headers;
  }

  private async postApproval(path: string, input: AgentApprovalRequest): Promise<Record<string, unknown>> {
    const response = await this.fetchWithSessionRetry(path, {
      method: "POST",
      body: JSON.stringify(input),
    });
    if (!response.ok) {
      throw new Error(`agent approval failed with status ${response.status}`);
    }
    return (await response.json()) as Record<string, unknown>;
  }

  private async fetchWithSessionRetry(path: string, init?: RequestInit): Promise<Response> {
    await this.ensureSession();
    let response = await fetch(`${this.baseUrl}${path}`, {
      ...init,
      headers: this.headers(),
    });
    if (response.status !== 401) {
      return response;
    }
    this.sessionToken = undefined;
    await this.ensureSession();
    response = await fetch(`${this.baseUrl}${path}`, {
      ...init,
      headers: this.headers(),
    });
    return response;
  }
}
