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
};

export type AgentSignResponse = {
  certificateId: string;
  challenge: string;
  bundle: AgentProofBundle;
};

export class DesktopAgentClient {
  constructor(private readonly baseUrl: string) {}

  async assertHealthy(): Promise<void> {
    const response = await fetch(`${this.baseUrl}/healthz`);
    if (!response.ok) {
      throw new Error(`agent health check failed with status ${response.status}`);
    }
  }

  async sign(input: AgentSignRequest): Promise<AgentSignResponse> {
    const response = await fetch(`${this.baseUrl}/sign`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(input),
    });
    if (!response.ok) {
      throw new Error(`agent sign failed with status ${response.status}`);
    }
    return (await response.json()) as AgentSignResponse;
  }
}
