import {
  DesktopAgentClient,
  type AgentProofBundle,
  type AgentSignRequest,
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
};

export type SignAndRunInput = {
  certificateId: string;
  password: string;
  facilityCode?: string;
  startMonth?: string;
  endMonth?: string;
  mode?: SyncMode;
};

export type RegisterStoredInput = {
  certificateId: string;
  password: string;
  facilityCode?: string;
};

export class CertdWebSdk {
  private readonly gateway: GatewayClient;
  private readonly agent: DesktopAgentClient;

  constructor(options: CertdWebSdkOptions) {
    this.gateway = new GatewayClient(trimTrailingSlash(options.gatewayBaseUrl));
    this.agent = new DesktopAgentClient(trimTrailingSlash(options.agentBaseUrl));
  }

  async createChallenge(
    organizationId: string,
    input: CreateChallengeInput,
  ): Promise<Challenge> {
    return this.gateway.createChallenge(organizationId, input);
  }

  async runEphemeralExecution(
    organizationId: string,
    input: SignAndRunInput,
  ): Promise<SyncRun> {
    await this.agent.assertHealthy();
    const challenge = await this.gateway.createChallenge(organizationId, {
      mode: "ephemeral",
      facilityCode: input.facilityCode,
    });
    const proof = await this.agent.sign(
      signRequestFrom(challenge.payload, input, organizationId),
    );
    return this.gateway.executeEphemeralExecution(organizationId, {
      challengeId: challenge.challengeId,
      challengePayload: challenge.payload,
      bundle: mergePassword(proof.bundle, input.password),
      facilityCode: input.facilityCode,
      startMonth: input.startMonth,
      endMonth: input.endMonth,
      mode: input.mode,
    });
  }

  async registerStoredCertificate(
    organizationId: string,
    input: RegisterStoredInput,
  ): Promise<StoredCertificate> {
    await this.agent.assertHealthy();
    const challenge = await this.gateway.createChallenge(organizationId, {
      mode: "stored",
      facilityCode: input.facilityCode,
    });
    const proof = await this.agent.sign(
      signRequestFrom(challenge.payload, input, organizationId),
    );
    return this.gateway.registerStoredCertificate(organizationId, {
      challengeId: challenge.challengeId,
      challengePayload: challenge.payload,
      bundle: mergePassword(proof.bundle, input.password),
    });
  }
}

function signRequestFrom(
  challenge: string,
  input: Pick<SignAndRunInput, "certificateId" | "password">,
  organizationId: string,
): AgentSignRequest {
  return {
    certificateId: input.certificateId,
    challenge,
    password: input.password,
    businessNo: organizationId,
  };
}

function mergePassword(bundle: AgentProofBundle, password: string) {
  return {
    ...bundle,
    password,
  };
}

function trimTrailingSlash(value: string): string {
  return value.replace(/\/+$/, "");
}

export type {
  AgentProofBundle,
  Challenge,
  CreateChallengeInput,
  StoredCertificate,
  SyncMode,
  SyncRun,
};
