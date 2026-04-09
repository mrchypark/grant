import assert from "node:assert/strict";
import test from "node:test";

import { CertdWebSdk } from "./index.ts";

type FetchCall = {
  input: RequestInfo | URL;
  init?: RequestInit;
};

function jsonResponse(value: unknown, status = 200): Response {
  return new Response(JSON.stringify(value), {
    status,
    headers: { "content-type": "application/json" },
  });
}

function textResponse(value: string, status = 200): Response {
  return new Response(value, { status });
}

function installFetchMock(handlers: Array<(call: FetchCall) => Response | Promise<Response>>) {
  const calls: FetchCall[] = [];
  const originalFetch = globalThis.fetch;
  let index = 0;

  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    calls.push({ input, init });
    const handler = handlers[index];
    if (!handler) {
      throw new Error(`unexpected fetch call #${index + 1}`);
    }
    index += 1;
    return handler({ input, init });
  }) as typeof fetch;

  return {
    calls,
    restore() {
      globalThis.fetch = originalFetch;
    },
  };
}

test("createChallenge() calls the gateway challenge endpoint", async () => {
  const mock = installFetchMock([
    () =>
      jsonResponse({
        challengeId: "challenge-1",
        mode: "ephemeral",
        nonce: "nonce-1",
        payload: "payload-1",
        expiresAt: "2026-04-09T00:00:00Z",
      }, 201),
  ]);

  try {
    const sdk = new CertdWebSdk({
      gatewayBaseUrl: "http://gateway.local",
      agentBaseUrl: "http://127.0.0.1:8765",
    });
    const challenge = await sdk.createChallenge("org-1", {
      mode: "ephemeral",
      facilityCode: "FAC-001",
    });

    assert.equal(challenge.challengeId, "challenge-1");
    assert.equal(mock.calls.length, 1);
    assert.equal(
      String(mock.calls[0].input),
      "http://gateway.local/api/v1/organizations/org-1/cert-gateway/challenges",
    );
  } finally {
    mock.restore();
  }
});

test("runEphemeralExecution() checks agent health, signs locally, then starts the crawl", async () => {
  const mock = installFetchMock([
    () => textResponse("ok"),
    () =>
      jsonResponse({
        challengeId: "challenge-ephemeral",
        mode: "ephemeral",
        nonce: "nonce-2",
        payload: "payload-ephemeral",
        expiresAt: "2026-04-09T00:00:00Z",
      }, 201),
    () =>
      jsonResponse({
        certificateId: "cert-1",
        challenge: "payload-ephemeral",
        bundle: {
          businessNo: "2768603291",
          password: "secret-password",
          commonName: "Test Certificate",
          signCertDerBase64: "AQID",
          signPriKeyBase64: "BAUG",
          kmCertDerBase64: "BwgJ",
          kmPriKeyBase64: "CgsM",
        },
      }),
    () =>
      jsonResponse(
        {
          id: "sync-1",
          organizationId: "org-1",
          plantId: "FAC-001",
          mode: "delta",
          status: "accepted",
          startedAt: null,
          finishedAt: null,
          durationMs: 0,
        },
        202,
      ),
  ]);

  try {
    const sdk = new CertdWebSdk({
      gatewayBaseUrl: "http://gateway.local",
      agentBaseUrl: "http://127.0.0.1:8765",
    });

    const run = await sdk.runEphemeralExecution("org-1", {
      certificateId: "cert-1",
      password: "secret-password",
      facilityCode: "FAC-001",
      startMonth: "2026-01",
      endMonth: "2026-03",
      mode: "delta",
    });

    assert.equal(run.status, "accepted");
    assert.equal(mock.calls.length, 4);
    assert.equal(String(mock.calls[0].input), "http://127.0.0.1:8765/healthz");
    assert.equal(String(mock.calls[2].input), "http://127.0.0.1:8765/sign");
    assert.equal(
      String(mock.calls[3].input),
      "http://gateway.local/api/v1/organizations/org-1/cert-gateway/ephemeral-executions",
    );
  } finally {
    mock.restore();
  }
});

test("registerStoredCertificate() signs locally and registers the encrypted certificate flow", async () => {
  const mock = installFetchMock([
    () => textResponse("ok"),
    () =>
      jsonResponse({
        challengeId: "challenge-stored",
        mode: "stored",
        nonce: "nonce-3",
        payload: "payload-stored",
        expiresAt: "2026-04-09T00:00:00Z",
      }, 201),
    () =>
      jsonResponse({
        certificateId: "cert-2",
        challenge: "payload-stored",
        bundle: {
          businessNo: "2768603291",
          password: "secret-password",
          commonName: "Stored Certificate",
          signCertDerBase64: "AQID",
          signPriKeyBase64: "BAUG",
          kmCertDerBase64: "BwgJ",
          kmPriKeyBase64: "CgsM",
        },
      }),
    () =>
      jsonResponse(
        {
          id: "stored-cert-1",
          organizationId: "org-1",
          businessNo: "2768603291",
          commonName: "Stored Certificate",
        },
        201,
      ),
  ]);

  try {
    const sdk = new CertdWebSdk({
      gatewayBaseUrl: "http://gateway.local",
      agentBaseUrl: "http://127.0.0.1:8765",
    });

    const certificate = await sdk.registerStoredCertificate("org-1", {
      certificateId: "cert-2",
      password: "secret-password",
      facilityCode: "FAC-002",
    });

    assert.equal(certificate.id, "stored-cert-1");
    assert.equal(mock.calls.length, 4);
    assert.equal(
      String(mock.calls[3].input),
      "http://gateway.local/api/v1/organizations/org-1/certificates",
    );
  } finally {
    mock.restore();
  }
});

test("runEphemeralExecution() rejects when the local agent health check fails", async () => {
  const mock = installFetchMock([() => textResponse("down", 503)]);

  try {
    const sdk = new CertdWebSdk({
      gatewayBaseUrl: "http://gateway.local",
      agentBaseUrl: "http://127.0.0.1:8765",
    });

    await assert.rejects(
      () =>
        sdk.runEphemeralExecution("org-1", {
          certificateId: "cert-1",
          password: "secret-password",
          facilityCode: "FAC-001",
        }),
      /agent health check failed/i,
    );
    assert.equal(mock.calls.length, 1);
  } finally {
    mock.restore();
  }
});
