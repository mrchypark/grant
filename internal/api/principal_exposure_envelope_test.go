package api

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"os"
	"runtime"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	pdppb "github.com/mrchypark/grant/api/pdp/v1"
	"github.com/mrchypark/grant/internal/grant"
	"google.golang.org/grpc"
)

var envelopeGrantFixtureSHA = "unpublished"

func TestPrincipalExposureJSONEnvelope(t *testing.T) {
	if os.Getenv("GRANT_EXPOSURE_JSON_ENVELOPE_INTEROP") != "1" {
		t.Skip("actual Exposure JSON envelope interop is opt-in")
	}
	if runtime.GOOS != "darwin" || runtime.GOARCH != "arm64" {
		t.Fatal("pinned Exposure artifact requires darwin/arm64")
	}
	const exposureSHA = "8c059c7e3f7c0d1137a1ab1f510842c049a5c16519776a9ad34310f5e7786b5a"
	if os.Getenv("GRANT_EXPOSURE_BINARY_SHA256") != exposureSHA {
		t.Fatal("use the pinned Exposure a1a2c287 artifact SHA-256")
	}
	binary := verifyPrincipalExposureBinary(t, os.Getenv("GRANT_EXPOSURE_BINARY"))
	t.Logf("Grant fixture source: %s", envelopeGrantFixtureSHA)
	ctx, cancel := context.WithTimeout(context.Background(), 70*time.Second)
	defer cancel()
	for _, transport := range []string{"http-pdp", "native-grpc-pdp"} {
		t.Run(transport, func(t *testing.T) {
			exercisePrincipalExposureJSONEnvelope(t, ctx, binary, transport == "native-grpc-pdp")
		})
	}
}

func exercisePrincipalExposureJSONEnvelope(t *testing.T, ctx context.Context, binary string, native bool) {
	t.Helper()
	// The proof is opaque fixture text. Deliberate whitespace, key order,
	// escape spelling and literal HTML-sensitive characters detect re-encoding.
	const envelope = "{\n  \"payload\" : { \"z\" : \"<tag>&\",\t\"a\" : \"\\u0061\" },\n  \"proof\": \"synthetic\"\n}\n"
	const mediaType = "application/json"
	const org = "fixture-owner"
	wantDigest := fmt.Sprintf("sha256:%x", sha256.Sum256([]byte(envelope)))
	f := newPrincipalExposureSystemWithAudience(t, ctx, nil, func(r *http.Request, token string) {
		body, err := io.ReadAll(io.LimitReader(r.Body, int64(len(envelope)+1)))
		if err != nil || string(body) != envelope || r.Method != http.MethodPost || r.URL.RequestURI() != "/items/allowed" {
			t.Errorf("upstream did not receive exact POST envelope: method=%s uri=%s bytes=%d err=%v", r.Method, r.URL.RequestURI(), len(body), err)
		}
		for name, want := range map[string]string{"Content-Type": mediaType, "X-Organization-ID": org, "Authorization": "Bearer " + token} {
			if values := r.Header.Values(name); len(values) != 1 || values[0] != want {
				t.Errorf("upstream did not receive original singleton %s", name)
			}
		}
		for _, name := range []string{"X-Grant-App-Auth-Key", "X-Grant-Master-Key"} {
			if len(r.Header.Values(name)) != 0 {
				t.Errorf("upstream received private header %s", name)
			}
		}
	}, true)
	digest := f.publishItem(t, "user", http.MethodPost, grant.HttpRequestPolicy{
		Body: "json", MaxBytes: 4096, ContentTypes: []string{mediaType},
		ForwardHeaders: []string{"content-type", "x-organization-id"}, Authorization: "forward-user",
	})
	var attempts, allows, denials, httpFallback atomic.Int64
	checkEvidence := func(raw []byte) string {
		var in grant.HttpCapabilityDecisionInput
		if err := json.Unmarshal(raw, &in); err != nil {
			t.Errorf("decode Exposure decision evidence: %v", err)
		} else if in.BodyDigest != wantDigest || in.BodyLength != uint64(len(envelope)) || in.WireVersion != 2 {
			t.Errorf("decision evidence differs from exact JSON envelope: digest=%s bytes=%d wire=%d", in.BodyDigest, in.BodyLength, in.WireVersion)
		}
		attempts.Add(1)
		return in.ResourceID
	}
	checkDecision := func(raw []byte, item string) {
		var out grant.HttpCapabilityDecision
		if err := json.Unmarshal(raw, &out); err != nil {
			t.Errorf("decode real Grant decision: %v", err)
			return
		}
		if out.ResourceType != "item" || out.ResourceID != item || out.CanonicalPath != "/items/"+item {
			t.Errorf("real Grant decision resource=%s/%s path=%q, want item/%s", out.ResourceType, out.ResourceID, out.CanonicalPath, item)
			return
		}
		switch {
		case item == "allowed" && out.Allowed:
			allows.Add(1)
		case item == "blocked" && !out.Allowed && out.Reason == "no_matching_binding":
			denials.Add(1)
		default:
			t.Errorf("unexpected real Grant decision for %q: allowed=%t reason=%q", item, out.Allowed, out.Reason)
		}
	}
	service := grant.NewService(f.config, f.store)
	realHTTP := New(service, false)
	pdpHTTP := principalExposureServer(t, ctx, http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if native {
			httpFallback.Add(1)
			w.WriteHeader(http.StatusServiceUnavailable)
			return
		}
		if r.Method == http.MethodPost && r.URL.Path == "/api/v2/http-capabilities/decide" {
			const limit = 2 * 1024 * 1024
			raw, err := io.ReadAll(io.LimitReader(r.Body, limit+1))
			if err != nil || len(raw) > limit {
				t.Errorf("invalid HTTP evidence read: bytes=%d err=%v", len(raw), err)
				w.WriteHeader(http.StatusBadRequest)
				return
			}
			item := checkEvidence(raw)
			_ = r.Body.Close()
			r.Body = io.NopCloser(bytes.NewReader(raw))
			reply := httptest.NewRecorder()
			realHTTP.ServeHTTP(reply, r)
			if reply.Code != http.StatusOK {
				t.Errorf("real Grant HTTP decision for %q returned status %d", item, reply.Code)
			} else {
				checkDecision(reply.Body.Bytes(), item)
			}
			for key, values := range reply.Header() {
				w.Header()[key] = values
			}
			w.WriteHeader(reply.Code)
			_, _ = w.Write(reply.Body.Bytes())
			return
		}
		realHTTP.ServeHTTP(w, r)
	}), false)
	settings := map[string]string{
		"GRANT_LEARNER_URL":              pdpHTTP.URL,
		"GOAUTHY_USER_CLIENT_ID":         f.config.Auth.Goauthy.ExpectedClientID,
		"GOAUTHY_USER_RESOURCE_AUDIENCE": f.config.Auth.Goauthy.Resource,
	}
	if native {
		listener, err := net.Listen("tcp", "127.0.0.1:0")
		if err != nil {
			t.Fatal(err)
		}
		server := grpc.NewServer(grpc.MaxRecvMsgSize(PDPGRPCMaxWireBytes), grpc.MaxSendMsgSize(PDPGRPCMaxWireBytes),
			grpc.WaitForHandlers(true),
			grpc.UnaryInterceptor(func(ctx context.Context, in any, info *grpc.UnaryServerInfo, next grpc.UnaryHandler) (any, error) {
				if info.FullMethod != pdppb.PolicyDecisionPoint_DecideHttp_FullMethodName {
					return next(ctx, in)
				}
				item := checkEvidence(in.(*pdppb.DecisionRequest).GetEvidenceJson())
				out, err := next(ctx, in)
				if reply, ok := out.(*pdppb.DecisionResponse); err == nil && ok && reply != nil {
					checkDecision(reply.GetDecisionJson(), item)
				} else {
					t.Errorf("real Grant gRPC decision for %q failed: %v", item, err)
				}
				return out, err
			}))
		RegisterPDPGRPC(server, service)
		done := make(chan error, 1)
		go func() { done <- server.Serve(listener) }()
		t.Cleanup(func() {
			server.Stop()
			_ = listener.Close()
			if err := <-done; err != nil && err != grpc.ErrServerStopped {
				t.Errorf("native PDP listener: %v", err)
			}
		})
		settings["GRANT_GRPC_TARGET"] = listener.Addr().String()
	}
	origin := f.startExposureWithPDP(t, binary, digest, false, settings)
	client := &http.Client{Timeout: 5 * time.Second, CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}
	t.Cleanup(client.CloseIdleConnections)
	for _, tc := range []struct {
		item        string
		status      int
		targetDelta int64
		decisions   *atomic.Int64
	}{{"allowed", http.StatusOK, 1, &allows}, {"blocked", http.StatusForbidden, 0, &denials}} {
		before, evaluated, hits := attempts.Load(), tc.decisions.Load(), f.upstreamHits.Load()
		req, err := http.NewRequestWithContext(ctx, http.MethodPost, origin+"/items/"+tc.item, strings.NewReader(envelope))
		if err != nil {
			t.Fatal(err)
		}
		req.Header.Set("Content-Type", mediaType)
		req.Header.Set("X-Organization-ID", org)
		req.Header.Set("Authorization", "Bearer "+f.tokens["user"])
		res, err := client.Do(req)
		if err != nil {
			t.Fatal(err)
		}
		_ = res.Body.Close()
		if res.StatusCode != tc.status || attempts.Load()-before != 1 || tc.decisions.Load()-evaluated != 1 || f.upstreamHits.Load()-hits != tc.targetDelta || httpFallback.Load() != 0 {
			t.Fatalf("%s: status=%d want=%d Grant attempts=%d evaluated=%d target calls=%d want=%d HTTP fallback=%d",
				tc.item, res.StatusCode, tc.status, attempts.Load()-before, tc.decisions.Load()-evaluated, f.upstreamHits.Load()-hits, tc.targetDelta, httpFallback.Load())
		}
		t.Logf("%s: status=%d Grant decisions=1 target calls=%d HTTP fallback=0", tc.item, res.StatusCode, tc.targetDelta)
	}
	f.assertNoApplications(t)
}
