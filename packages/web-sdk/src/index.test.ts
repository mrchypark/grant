import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import test from "node:test";

import {
  WebSdk,
  configureHtml5CertificateInput,
  findHtml5CertificateFiles,
  prepareHtml5CertificateBundle,
  prepareHtml5ChallengeProof,
} from "./index.ts";
import { configureNpkiWasmAssets } from "./npki-wasm.ts";

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
    const sdk = new WebSdk({
      gatewayBaseUrl: "https://gateway.local",
      agentBaseUrl: "http://127.0.0.1:8765",
    });
    const challenge = await sdk.createChallenge("org-1", {
      mode: "ephemeral",
    });

    assert.equal(challenge.challengeId, "challenge-1");
    assert.equal(mock.calls.length, 1);
    assert.equal(
      String(mock.calls[0].input),
      "https://gateway.local/api/v1/organizations/org-1/cert-gateway/challenges",
    );
  } finally {
    mock.restore();
  }
});

test("createChallenge() allows IPv6 loopback gateway URLs over http", async () => {
  const mock = installFetchMock([
    () =>
      jsonResponse({
        challengeId: "challenge-ipv6",
        mode: "stored",
        nonce: "nonce-ipv6",
        payload: "payload-ipv6",
        expiresAt: "2026-04-09T00:00:00Z",
      }, 201),
  ]);

  try {
    const sdk = new WebSdk({
      gatewayBaseUrl: "http://[::1]:4173",
      agentBaseUrl: "http://127.0.0.1:8765",
    });

    const challenge = await sdk.createChallenge("org-1", {
      mode: "stored",
    });

    assert.equal(challenge.challengeId, "challenge-ipv6");
    assert.equal(
      String(mock.calls[0].input),
      "http://[::1]:4173/api/v1/organizations/org-1/cert-gateway/challenges",
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
    const sdk = new WebSdk({
      gatewayBaseUrl: "https://gateway.local",
      agentBaseUrl: "http://127.0.0.1:8765",
    });

    await sdk.assertAvailable();

    assert.equal(mock.calls.length, 1);
    assert.equal(String(mock.calls[0].input), "http://127.0.0.1:8765/healthz");
  } finally {
    mock.restore();
  }
});

test("raw.agent.listCertificates() preserves extended certificate metadata", async () => {
  const mock = installFetchMock([
    () =>
      jsonResponse({
        sessionToken: "agent-session-cert-list",
        expiresAt: "2026-04-09T00:05:00Z",
      }, 201),
    () =>
      jsonResponse({
        items: [
          {
            id: "pkcs12:/Volumes/Lexar/protected.p12",
            commonName: "Protected PKCS12 Certificate",
            businessNo: "2768603291",
            notAfter: "2027-04-09T00:00:00Z",
            sourceKind: "pkcs12",
            storageKind: "pkcs12",
            displayPath: "protected.p12",
            issuerName: "Test Issuer",
            hasKmMaterial: false,
          },
        ],
      }),
  ]);

  try {
    const sdk = new WebSdk({
      gatewayBaseUrl: "https://gateway.local",
      agentBaseUrl: "http://127.0.0.1:8765",
    });

    const result = await sdk.raw.agent.listCertificates();

    assert.equal(mock.calls.length, 2);
    assert.equal(String(mock.calls[0].input), "http://127.0.0.1:8765/sessions/open");
    assert.equal(String(mock.calls[1].input), "http://127.0.0.1:8765/certificates");
    assert.deepEqual(result.items[0], {
      id: "pkcs12:/Volumes/Lexar/protected.p12",
      commonName: "Protected PKCS12 Certificate",
      businessNo: "2768603291",
      notAfter: "2027-04-09T00:00:00Z",
      sourceKind: "pkcs12",
      storageKind: "pkcs12",
      displayPath: "protected.p12",
      issuerName: "Test Issuer",
      hasKmMaterial: false,
    });
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
    const sdk = new WebSdk({
      gatewayBaseUrl: "https://gateway.local",
      agentBaseUrl: "http://127.0.0.1:8765",
    });

    const run = await sdk.runEphemeral("org-1", { businessNo: "2768603291" });

    assert.equal(run.status, "completed");
    assert.equal(mock.calls.length, 3);
    assert.equal(String(mock.calls[0].input), "http://127.0.0.1:8765/healthz");
    assert.equal(String(mock.calls[1].input), "http://127.0.0.1:8765/sessions/open");
    assert.equal(String(mock.calls[2].input), "http://127.0.0.1:8765/approve-ephemeral-run");
    assert.deepEqual(JSON.parse(String(mock.calls[2].init?.body)), {
      gatewayBaseUrl: "https://gateway.local",
      organizationId: "org-1",
      businessNo: "2768603291",
    });
    assert.equal((mock.calls[2].init?.headers as Record<string, string>)["x-agent-session"], "agent-session-1");
  } finally {
    mock.restore();
  }
});

test("runEphemeral() forwards optional sync scope to the local agent", async () => {
  const mock = installFetchMock([
    () => textResponse("ok"),
    () =>
      jsonResponse({
        sessionToken: "agent-session-scope",
        expiresAt: "2026-04-09T00:05:00Z",
      }, 201),
    () =>
      jsonResponse({
        id: "sync-scope-1",
        organizationId: "org-1",
        plantId: "FAC-009",
        mode: "delta",
        status: "completed",
        startedAt: null,
        finishedAt: null,
        durationMs: 0,
      }, 202),
  ]);

  try {
    const sdk = new WebSdk({
      gatewayBaseUrl: "https://gateway.local",
      agentBaseUrl: "http://127.0.0.1:8765",
    });

    await sdk.runEphemeral("org-1", {
      businessNo: "2768603291",
      facilityCode: " FAC-009 ",
      startMonth: " 2026-01 ",
      endMonth: " 2026-03 ",
      mode: "delta",
    });

    assert.deepEqual(JSON.parse(String(mock.calls[2].init?.body)), {
      gatewayBaseUrl: "https://gateway.local",
      organizationId: "org-1",
      businessNo: "2768603291",
      facilityCode: "FAC-009",
      startMonth: "2026-01",
      endMonth: "2026-03",
      mode: "delta",
    });
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
    const sdk = new WebSdk({
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
    const sdk = new WebSdk({
      gatewayBaseUrl: "https://gateway.local",
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

test("runEphemeral() allows omitting businessNo so the server can use the linked organization number", async () => {
  const mock = installFetchMock([
    () => textResponse("ok"),
    () =>
      jsonResponse({
        sessionToken: "agent-session-omitted-business-no",
        expiresAt: "2026-04-09T00:05:00Z",
      }, 201),
    () =>
      jsonResponse({
        id: "sync-omitted-business-no",
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
    const sdk = new WebSdk({
      gatewayBaseUrl: "https://gateway.local",
      agentBaseUrl: "http://127.0.0.1:8765",
    });

    const run = await sdk.runEphemeral("org-1", {});

    assert.equal(run.id, "sync-omitted-business-no");
    assert.deepEqual(JSON.parse(String(mock.calls[2].init?.body)), {
      gatewayBaseUrl: "https://gateway.local",
      organizationId: "org-1",
    });
  } finally {
    mock.restore();
  }
});

test("runEphemeral() reopens the local agent session after a 401 response", async () => {
  const mock = installFetchMock([
    () => textResponse("ok"),
    () =>
      jsonResponse({
        sessionToken: "expired-session",
        expiresAt: "2026-04-09T00:05:00Z",
      }, 201),
    () => textResponse("expired", 401),
    () =>
      jsonResponse({
        sessionToken: "fresh-session",
        expiresAt: "2026-04-09T00:10:00Z",
      }, 201),
    () =>
      jsonResponse({
        id: "sync-401-recovered",
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
    const sdk = new WebSdk({
      gatewayBaseUrl: "https://gateway.local",
      agentBaseUrl: "http://127.0.0.1:8765",
    });

    const run = await sdk.runEphemeral("org-1", { businessNo: "2768603291" });

    assert.equal(run.id, "sync-401-recovered");
    assert.equal(mock.calls.length, 5);
    assert.equal(String(mock.calls[3].input), "http://127.0.0.1:8765/sessions/open");
    assert.equal(String(mock.calls[4].input), "http://127.0.0.1:8765/approve-ephemeral-run");
    assert.equal((mock.calls[2].init?.headers as Record<string, string>)["x-agent-session"], "expired-session");
    assert.equal((mock.calls[4].init?.headers as Record<string, string>)["x-agent-session"], "fresh-session");
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
    const sdk = new WebSdk({
      gatewayBaseUrl: "https://gateway.local",
      agentBaseUrl: "http://127.0.0.1:8765",
    });

    const run = await sdk.runStored("org-1", {
      certificateId: "stored-cert-1",
    });

    assert.equal(run.id, "sync-stored-1");
    assert.equal(mock.calls.length, 2);
    assert.equal(
      String(mock.calls[0].input),
      "https://gateway.local/api/v1/organizations/org-1/certificates/stored-cert-1/crawl-tokens",
    );
    assert.equal(
      String(mock.calls[1].input),
      "https://gateway.local/api/v1/organizations/org-1/crawls",
    );
  } finally {
    mock.restore();
  }
});

test("runEphemeral() rejects when the local agent health check fails", async () => {
  const mock = installFetchMock([() => textResponse("down", 503)]);

  try {
    const sdk = new WebSdk({
      gatewayBaseUrl: "https://gateway.local",
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

test("browser example defaults the agent base URL to the loopback desktop agent", async () => {
  const htmlPath = fileURLToPath(new URL("../examples/browser/index.html", import.meta.url));
  const html = await readFile(htmlPath, "utf8");

  assert.match(
    html,
    /<input id="agentBaseUrl" value="http:\/\/127\.0\.0\.1:8765" \/>/,
  );
});

test("findHtml5CertificateFiles() matches the required certificate bundle files by basename", () => {
  const selection = findHtml5CertificateFiles([
    fileWithPath("NPKI/yessign/USER/cn=test/signCert.der", "sign-cert"),
    fileWithPath("NPKI/yessign/USER/cn=test/signPri.key", "sign-key"),
    fileWithPath("NPKI/yessign/USER/cn=test/kmCert.der", "km-cert"),
    fileWithPath("NPKI/yessign/USER/cn=test/kmPri.key", "km-key"),
  ]);

  assert.equal(selection.fileNames.signCertDer, "NPKI/yessign/USER/cn=test/signCert.der");
  assert.equal(selection.files.kmPriKey.name, "kmPri.key");
});

test("findHtml5CertificateFiles() rejects missing files", () => {
  assert.throws(
    () =>
      findHtml5CertificateFiles([
        fileWithPath("signCert.der", "sign-cert"),
        fileWithPath("signPri.key", "sign-key"),
        fileWithPath("kmCert.der", "km-cert"),
      ]),
    /missing certificate file: kmPri\.key/i,
  );
});

test("findHtml5CertificateFiles() rejects a multi-certificate directory selection", () => {
  assert.throws(
    () =>
      findHtml5CertificateFiles([
        fileWithPath("NPKI/yessign/USER/cn=beta/signCert.der", "beta-sign-cert"),
        fileWithPath("NPKI/yessign/USER/cn=alpha/signCert.der", "alpha-sign-cert"),
        fileWithPath("NPKI/yessign/USER/cn=beta/signPri.key", "beta-sign-key"),
        fileWithPath("NPKI/yessign/USER/cn=alpha/signPri.key", "alpha-sign-key"),
        fileWithPath("NPKI/yessign/USER/cn=beta/kmCert.der", "beta-km-cert"),
        fileWithPath("NPKI/yessign/USER/cn=alpha/kmCert.der", "alpha-km-cert"),
        fileWithPath("NPKI/yessign/USER/cn=beta/kmPri.key", "beta-km-key"),
        fileWithPath("NPKI/yessign/USER/cn=alpha/kmPri.key", "alpha-km-key"),
      ]),
    /multiple certificate bundles selected/i,
  );
});

test("prepareHtml5CertificateBundle() recovers after a transient wasm asset load failure", async () => {
  try {
    configureNpkiWasmAssets({
      wasmExecUrl: new URL("./missing-wasm-exec.js", import.meta.url),
      wasmUrl: new URL("./generated/npki_signer.wasm", import.meta.url),
    });

    await assert.rejects(
      () =>
        prepareHtml5CertificateBundle(
          [
            fileWithBytes("signCert.der", base64Bytes(FIXTURE_SIGN_CERT_DER_BASE64)),
            fileWithPath("signPri.key", "SK"),
            fileWithPath("kmCert.der", "KC"),
            fileWithPath("kmPri.key", "KK"),
          ],
          {
            businessNo: "2768603291",
            commonName: "Transient Failure Certificate",
            password: "secret-password",
          },
        ),
      /ENOENT|failed to load/i,
    );
  } finally {
    configureNpkiWasmAssets({
      wasmExecUrl: new URL("./generated/wasm_exec.js", import.meta.url),
      wasmUrl: new URL("./generated/npki_signer.wasm", import.meta.url),
    });
  }

  const prepared = await prepareHtml5CertificateBundle(
    [
      fileWithBytes("signCert.der", base64Bytes(FIXTURE_SIGN_CERT_DER_BASE64)),
      fileWithPath("signPri.key", "SK"),
      fileWithPath("kmCert.der", "KC"),
      fileWithPath("kmPri.key", "KK"),
    ],
    {
      businessNo: "2768603291",
      commonName: "Recovered Certificate",
      password: "secret-password",
    },
  );

  assert.equal(prepared.bundle.commonName, "Fixture Certificate");
  assert.equal(prepared.bundle.businessNo, "2768603291");
});

test("prepareHtml5CertificateBundle() derives certificate identity from signCert.der", async () => {
  const prepared = await prepareHtml5CertificateBundle(
    [
      fileWithBytes("signCert.der", base64Bytes(FIXTURE_SIGN_CERT_DER_BASE64)),
      fileWithPath("signPri.key", "SK"),
      fileWithPath("kmCert.der", "KC"),
      fileWithPath("kmPri.key", "KK"),
    ],
    {
      businessNo: " 2768603291 ",
      commonName: " Test Certificate ",
      password: " secret-password ",
    },
  );

  assert.deepEqual(prepared.selection.fileNames, {
    signCertDer: "signCert.der",
    signPriKey: "signPri.key",
    kmCertDer: "kmCert.der",
    kmPriKey: "kmPri.key",
  });
  assert.equal(prepared.bundle.businessNo, "2768603291");
  assert.equal(prepared.bundle.commonName, "Fixture Certificate");
  assert.equal(prepared.bundle.password, "secret-password");
  assert.equal(prepared.bundle.signCertDerBase64, FIXTURE_SIGN_CERT_DER_BASE64);
  assert.equal(prepared.bundle.signPriKeyBase64, "U0s=");
  assert.equal(prepared.bundle.kmCertDerBase64, "S0M=");
  assert.equal(prepared.bundle.kmPriKeyBase64, "S0s=");
});

test("configureHtml5CertificateInput() configures a directory-capable picker", () => {
  const input = createInputMock();
  configureHtml5CertificateInput(input as never, { mode: "directory" });

  assert.equal(input.type, "file");
  assert.equal(input.multiple, true);
  assert.equal(input.accept, ".der,.key,application/octet-stream");
  assert.equal(input.attributes.get("webkitdirectory"), "");
  assert.equal(input.attributes.get("directory"), "");
});

test("prepareHtml5ChallengeProof() decrypts signPri.key and signs the gateway challenge", async () => {
  const proof = await prepareHtml5ChallengeProof(
    "cert-gateway:test",
    [
      fileWithBytes("signCert.der", base64Bytes(FIXTURE_SIGN_CERT_DER_BASE64)),
      fileWithBytes("signPri.key", base64Bytes(FIXTURE_SIGN_PRI_BASE64)),
      fileWithBytes("kmCert.der", Uint8Array.of(4, 5, 6)),
      fileWithBytes("kmPri.key", Uint8Array.of(7, 8, 9)),
    ],
    {
      commonName: "Caller Provided Name",
      businessNo: "1111111111",
      password: "test-password",
    },
  );

  assert.equal(proof.bundle.commonName, "Fixture Certificate");
  assert.equal(proof.bundle.businessNo, "2768603291");
  assert.equal(proof.challengeSignatureBase64, FIXTURE_SIGNATURE_BASE64);
});

test("runEphemeralWithBrowserCertificate() requires explicit escrow opt-in", async () => {
  const sdk = new WebSdk({
    gatewayBaseUrl: "https://gateway.local",
    agentBaseUrl: "http://127.0.0.1:8765",
    activeOrganizationId: "org-1",
  });

  await assert.rejects(
    () =>
      sdk.runEphemeralWithBrowserCertificate("org-1", {
        certificateFiles: [fileWithBytes("signCert.der", base64Bytes(FIXTURE_SIGN_CERT_DER_BASE64))],
        password: "test-password",
      }),
    /Escrow upload is an explicit opt-in/i,
  );
});

test("createSignedChallengeProof() signs in-browser without uploading escrow secrets", async () => {
  const mock = installFetchMock([
    () =>
      jsonResponse({
        challengeId: "browser-challenge-proof-1",
        mode: "ephemeral",
        nonce: "nonce-browser-proof-1",
        payload: "cert-gateway:test",
        expiresAt: "2026-04-20T00:00:00Z",
      }, 201),
  ]);

  try {
    const sdk = new WebSdk({
      gatewayBaseUrl: "https://gateway.local",
      agentBaseUrl: "http://127.0.0.1:8765",
      activeOrganizationId: "org-1",
    });

    const proof = await sdk.createSignedChallengeProof("org-1", {
      certificateFiles: [
        fileWithBytes("signCert.der", base64Bytes(FIXTURE_SIGN_CERT_DER_BASE64)),
        fileWithBytes("signPri.key", base64Bytes(FIXTURE_SIGN_PRI_BASE64)),
        fileWithBytes("kmCert.der", Uint8Array.of(4, 5, 6)),
        fileWithBytes("kmPri.key", Uint8Array.of(7, 8, 9)),
      ],
      password: "test-password",
      mode: "ephemeral",
      facilityCode: "FAC-101",
    });

    assert.equal(proof.challengeId, "browser-challenge-proof-1");
    assert.equal(proof.mode, "ephemeral");
    assert.equal(proof.certificate.commonName, "Fixture Certificate");
    assert.equal(proof.certificate.businessNo, "2768603291");
    assert.equal(proof.challengeSignatureBase64, FIXTURE_SIGNATURE_BASE64);
    assert.equal(mock.calls.length, 1);
  } finally {
    mock.restore();
  }
});

test("runEphemeralWithEscrowedBrowserCertificate() creates a challenge and executes the gateway flow without the agent", async () => {
  const mock = installFetchMock([
    () =>
      jsonResponse({
        challengeId: "browser-challenge-1",
        mode: "ephemeral",
        nonce: "nonce-browser-1",
        payload: "cert-gateway:test",
        expiresAt: "2026-04-20T00:00:00Z",
      }, 201),
    ({ init }) => {
      const body = JSON.parse(String(init?.body));
      assert.equal(body.challengeSignatureBase64, FIXTURE_SIGNATURE_BASE64);
      assert.equal(body.bundle.commonName, "Fixture Certificate");
      assert.equal(body.bundle.businessNo, "2768603291");
      assert.equal(body.bundle.password, "test-password");
      return jsonResponse({
        id: "browser-sync-1",
        organizationId: "org-1",
        plantId: "FAC-101",
        mode: "full",
        status: "completed",
        startedAt: null,
        finishedAt: null,
        durationMs: 777,
      }, 202);
    },
  ]);

  try {
    const sdk = new WebSdk({
      gatewayBaseUrl: "https://gateway.local",
      agentBaseUrl: "http://127.0.0.1:8765",
      activeOrganizationId: "org-1",
    });

    const run = await sdk.runEphemeralWithEscrowedBrowserCertificate("org-1", {
      certificateFiles: [
        fileWithBytes("signCert.der", base64Bytes(FIXTURE_SIGN_CERT_DER_BASE64)),
        fileWithBytes("signPri.key", base64Bytes(FIXTURE_SIGN_PRI_BASE64)),
        fileWithBytes("kmCert.der", Uint8Array.of(4, 5, 6)),
        fileWithBytes("kmPri.key", Uint8Array.of(7, 8, 9)),
      ],
      commonName: "Caller Provided Name",
      businessNo: "1111111111",
      password: "test-password",
      facilityCode: " FAC-101 ",
      mode: "full",
      escrow: true,
    });

    assert.equal(run.id, "browser-sync-1");
    assert.equal(mock.calls.length, 2);
    assert.equal(
      String(mock.calls[0].input),
      "https://gateway.local/api/v1/organizations/org-1/cert-gateway/challenges",
    );
    assert.equal(
      String(mock.calls[1].input),
      "https://gateway.local/api/v1/organizations/org-1/cert-gateway/ephemeral-executions",
    );
  } finally {
    mock.restore();
  }
});

function fileWithPath(path: string, contents: string): File {
  const file = new File([contents], path.split(/[\\/]/).pop() ?? path);
  Object.defineProperty(file, "webkitRelativePath", {
    configurable: true,
    value: path,
  });
  return file;
}

function fileWithBytes(path: string, contents: Uint8Array): File {
  const file = new File([contents], path.split(/[\\/]/).pop() ?? path);
  Object.defineProperty(file, "webkitRelativePath", {
    configurable: true,
    value: path,
  });
  return file;
}

function base64Bytes(value: string): Uint8Array {
  return new Uint8Array(Buffer.from(value, "base64"));
}

function createInputMock() {
  const attributes = new Map<string, string>();

  return {
    accept: "",
    attributes,
    multiple: false,
    removeAttribute(name: string) {
      attributes.delete(name);
    },
    setAttribute(name: string, value: string) {
      attributes.set(name, value);
    },
    type: "",
  };
}

const FIXTURE_SIGN_CERT_DER_BASE64 = "MIIDRzCCAi+gAwIBAgIUV9JHDCOgsvNYeKYWamQjNMZnaxMwDQYJKoZIhvcNAQELBQAwMzEcMBoGA1UEAwwTRml4dHVyZSBDZXJ0aWZpY2F0ZTETMBEGA1UEBRMKMjc2ODYwMzI5MTAeFw0yNjA0MTcxNTU2MzlaFw0zNjA0MTQxNTU2MzlaMDMxHDAaBgNVBAMME0ZpeHR1cmUgQ2VydGlmaWNhdGUxEzARBgNVBAUTCjI3Njg2MDMyOTEwggEiMA0GCSqGSIb3DQEBAQUAA4IBDwAwggEKAoIBAQCVSfppz/IffKtiPnJeFNIlJo8tasq0FMf+o4yFN7ngVgoYOK92C04KzHvwUDAgsAgDC6wuTXALkiyuNh4unQDAEgtMC3XsYNRcH/stwSWzN+Tj0QV9rvlwx+GqFA4wbmKAqkZltOn23/w0OjisHbuO77y9rpOTAIQA4eFmeghHAHlTNcBMATJzRzLEnN7/PSjbY6CgWfxfQqAMfRF3blmeUZRsqFfMdF/qMwlE4A1SaONC8fzbhZOKic2FK3ut1vkMkH3rpB54TIMbryS3jq2WGVt1WPcHP+g1sJYPmxCD0DPsoYLPiiK32W7wlV437kpt/PhYulknZzO2Vwd3tcXzAgMBAAGjUzBRMB0GA1UdDgQWBBTQHJH/9G4mMNWgM7Xlik6+/Bi1CDAfBgNVHSMEGDAWgBTQHJH/9G4mMNWgM7Xlik6+/Bi1CDAPBgNVHRMBAf8EBTADAQH/MA0GCSqGSIb3DQEBCwUAA4IBAQAKlyny8yeeyN5trGSZNm9beIPhkuLoXQjN28km80hyZMcJ1jFF72zktFQbDtV1NOCyFr4wCkJQHs5/pbJTSgYVbGdP4VcWnKVIfsYhaYDhVnfaI2jNfM/fWnYbtPjxcMn1WJkxh3z/Z8/AE0qBF2QcPJZkPFdrEOfP87nEYzKiaR21gq2kZQokdeqLQ7L8+iD/Hd1OZjKRk7r470OayRO3Ti0tnTUgbxB3AUvN2p4GZEyARCy42I+eQ+PQCe91A4HREBuuzQAmZ6J1V8lIa7rlv1LgrltCtYNmidm5tp3YKi/wBhPDaMuDqsIrMrFOByRwJdaQ4VdpvCQplOR4s94L";
const FIXTURE_SIGN_PRI_BASE64 = "MIICoDAaBggqgxqMmkQBBDAOBAgxMjM0NTY3OAICCAAEggKApebdP5DRURHQ361TvwbGbJOTWJ/zh8se0cpWIUGLjzqMV1/Uo946hDBmDcs18AonVhsQ7n/7wdhZLWu17KhwXMsXGty82Qkq6l/mA5nrjyEz5kdTULcxEoVnovi9/GIvhetbwzF8TChyrtm/dhvy5sRVPZG6zMRM90fCmA7+9YnDKy+HKoy/T7oHu6beXf1UC3eZ70kMEes/UPcrvNPNESW9vFxOzG0JW861a46yzOlg8xcvnBS3WSxWsSzF5OZ7r8lVCntod5z/4+i9oGdBzTw13BVXW1Zc0ZILxr9soCdeC83Fov8qCTuyvRq3rLKTr0QXpc7AaS1PaAwATrrERCAxbjmSS+jDUgvmaXkNZ9XCMRztNF0qh/+YH4fV4byi291vJh8WTM8ITirONhCBSKgnSQOkacR3F8gh2ECySj4jp4R2JZTUPF9ksAWw2poVlaLEHDiCh/mq6wKQeW44oUGjZ/DoEAhqe0KgFsP6+us1HtW8/NQ2htrDhGWRwvmW5bUUdwSODTqHzitoHkfBDuc8/KkWPBHsxks6r8HOeqn867FC69mDcRvKsyD8W+RbX0wfSOd7JDv3dxWB4o68wsEOWMp60GVUz4MvsED1W5ZB4LAfxXY9K6pl0qjPwmXJ8eEWeCgcs45H7De904tPMU1WBZaNyabPECpd2u14oawP/xgrQGg5egrOpWsYWsfpIWQD+D5aAJo40ztZ64asZOjsDir1TKX/KfTHYfkGr52ue3rkJRJWFKieQUxw9izIp+AHjzQJBI7wHEYvvJfsHpRU6nNcoX2mC82jtPh5V4hAiPMbEgS/jig5Fp4fc+UzCV4DrcnLQfPm7OnDv1PgRQ==";
const FIXTURE_SIGNATURE_BASE64 = "DaFD/3vONkiwCP9n9kU7Ezi6J//L+K4TEsJaxlb5NTY4vn7jjLegmNtL9UeGDOiShmCbmZ7xGYreDVOdy/juQQXezrvZ45D043LYCeAm9woEu6zL6+nGbN1IXuHzapd2s5TN1aYjZPS5BvFYlMa5NmTzueJaXvv99pS7ye5VFXY=";
