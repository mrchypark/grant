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

function installLocationMock(href: string) {
  const original = Object.getOwnPropertyDescriptor(globalThis, "location");
  Object.defineProperty(globalThis, "location", {
    configurable: true,
    value: new URL(href),
  });
  return {
    restore() {
      if (original) {
        Object.defineProperty(globalThis, "location", original);
        return;
      }
      delete (globalThis as { location?: Location }).location;
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

test("assertAvailable() checks agent health", async () => {
  const mock = installFetchMock([
    () => textResponse("ok"),
  ]);

  try {
    const sdk = new CertdWebSdk({
      gatewayBaseUrl: "http://gateway.local",
      agentBaseUrl: "http://127.0.0.1:8765",
    });

    await sdk.assertAvailable();

    assert.equal(mock.calls.length, 1);
    assert.equal(String(mock.calls[0].input), "http://127.0.0.1:8765/healthz");
  } finally {
    mock.restore();
  }
});

test("runEphemeral() checks agent health then delegates approval and execution to the local agent", async () => {
  const mock = installFetchMock([
    () => textResponse("ok"),
    () =>
      jsonResponse({
        sessionToken: "agent-session-1",
        expiresAt: "2026-04-09T00:05:00Z",
      }, 201),
    () =>
      jsonResponse({
        id: "sync-1",
        organizationId: "org-1",
        plantId: "FAC-001",
        mode: "full",
        status: "completed",
        startedAt: null,
        finishedAt: null,
        durationMs: 0,
      }, 202),
  ]);

  try {
    const sdk = new CertdWebSdk({
      gatewayBaseUrl: "http://gateway.local",
      agentBaseUrl: "http://127.0.0.1:8765",
    });

    const run = await sdk.runEphemeral("org-1", { businessNo: "2768603291" });

    assert.equal(run.status, "completed");
    assert.equal(mock.calls.length, 3);
    assert.equal(String(mock.calls[0].input), "http://127.0.0.1:8765/healthz");
    assert.equal(String(mock.calls[1].input), "http://127.0.0.1:8765/sessions/open");
    assert.equal(String(mock.calls[2].input), "http://127.0.0.1:8765/approve-ephemeral-run");
    assert.deepEqual(JSON.parse(String(mock.calls[2].init?.body)), {
      gatewayBaseUrl: "http://gateway.local",
      organizationId: "org-1",
      businessNo: "2768603291",
    });
    assert.equal((mock.calls[2].init?.headers as Record<string, string>)["x-agent-session"], "agent-session-1");
  } finally {
    mock.restore();
  }
});

test("runEphemeral() resolves a relative gateway base URL before sending it to the local agent", async () => {
  const locationMock = installLocationMock("http://127.0.0.1:4173/");
  const mock = installFetchMock([
    () => textResponse("ok"),
    () =>
      jsonResponse({
        sessionToken: "agent-session-relative",
        expiresAt: "2026-04-09T00:05:00Z",
      }, 201),
    () =>
      jsonResponse({
        id: "sync-relative-1",
        organizationId: "org-1",
        plantId: "FAC-001",
        mode: "full",
        status: "completed",
        startedAt: null,
        finishedAt: null,
        durationMs: 0,
      }, 202),
  ]);

  try {
    const sdk = new CertdWebSdk({
      gatewayBaseUrl: "/gateway",
      agentBaseUrl: "http://127.0.0.1:8765",
    });

    await sdk.runEphemeral("org-1", { businessNo: "2768603291" });

    assert.deepEqual(JSON.parse(String(mock.calls[2].init?.body)), {
      gatewayBaseUrl: "http://127.0.0.1:4173/gateway",
      organizationId: "org-1",
      businessNo: "2768603291",
    });
  } finally {
    mock.restore();
    locationMock.restore();
  }
});

test("registerStored() delegates approval and registration to the local agent", async () => {
  const mock = installFetchMock([
    () => textResponse("ok"),
    () =>
      jsonResponse({
        sessionToken: "agent-session-2",
        expiresAt: "2026-04-09T00:05:00Z",
      }, 201),
    () =>
      jsonResponse({
        id: "stored-cert-1",
        organizationId: "org-1",
        businessNo: "2768603291",
        commonName: "Stored Certificate",
      }, 201),
  ]);

  try {
    const sdk = new CertdWebSdk({
      gatewayBaseUrl: "http://gateway.local",
      agentBaseUrl: "http://127.0.0.1:8765",
    });

    const certificate = await sdk.registerStored("org-1", { businessNo: "2768603291" });

    assert.equal(certificate.id, "stored-cert-1");
    assert.equal(mock.calls.length, 3);
    assert.equal(String(mock.calls[2].input), "http://127.0.0.1:8765/approve-stored-registration");
    assert.equal((mock.calls[2].init?.headers as Record<string, string>)["x-agent-session"], "agent-session-2");
  } finally {
    mock.restore();
  }
});

test("runStored() issues a crawl task token and starts the stored crawl", async () => {
  const mock = installFetchMock([
    () =>
      jsonResponse(
        {
          crawlTaskToken: "crawl-token-1",
          expiresAt: "2026-04-09T00:01:00Z",
        },
        201,
      ),
    () =>
      jsonResponse(
        {
          id: "sync-stored-1",
          organizationId: "org-1",
          plantId: "FAC-003",
          mode: "delta",
          status: "completed",
          startedAt: null,
          finishedAt: null,
          durationMs: 1234,
        },
        202,
      ),
  ]);

  try {
    const sdk = new CertdWebSdk({
      gatewayBaseUrl: "http://gateway.local",
      agentBaseUrl: "http://127.0.0.1:8765",
    });

    const run = await sdk.runStored("org-1", {
      certificateId: "stored-cert-1",
    });

    assert.equal(run.id, "sync-stored-1");
    assert.equal(mock.calls.length, 2);
    assert.equal(
      String(mock.calls[0].input),
      "http://gateway.local/api/v1/organizations/org-1/certificates/stored-cert-1/crawl-tokens",
    );
    assert.equal(
      String(mock.calls[1].input),
      "http://gateway.local/api/v1/organizations/org-1/crawls",
    );
  } finally {
    mock.restore();
  }
});

test("runEphemeral() rejects when businessNo is missing", async () => {
  const mock = installFetchMock([() => textResponse("ok")]);

  try {
    const sdk = new CertdWebSdk({
      gatewayBaseUrl: "http://gateway.local",
      agentBaseUrl: "http://127.0.0.1:8765",
    });

    await assert.rejects(
      () => sdk.runEphemeral("org-1", { businessNo: "" }),
      /businessNo is required/i,
    );
    assert.equal(mock.calls.length, 1);
  } finally {
    mock.restore();
  }
});

test("runEphemeral() rejects when the local agent health check fails", async () => {
  const mock = installFetchMock([() => textResponse("down", 503)]);

  try {
    const sdk = new CertdWebSdk({
      gatewayBaseUrl: "http://gateway.local",
      agentBaseUrl: "http://127.0.0.1:8765",
    });

    await assert.rejects(
      () =>
        sdk.runEphemeral("org-1", {
          certificateId: "cert-1",
          password: "secret-password",
        }),
      /agent health check failed/i,
    );
    assert.equal(mock.calls.length, 1);
  } finally {
    mock.restore();
  }
});
