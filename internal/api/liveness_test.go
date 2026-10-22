package api

import (
	"net/http/httptest"
	"testing"

	"github.com/mrchypark/grant/internal/grant"
)

func TestLivenessDoesNotContactDatabase(t *testing.T) {
	// No Store exists: contacting storage would panic. Both node roles must
	// remain alive during a database outage instead of entering restart loops.
	for _, learner := range []bool{false, true} {
		server := New(grant.NewService(grant.Config{}, nil), learner)
		recorder := httptest.NewRecorder()
		server.ServeHTTP(recorder, httptest.NewRequest("GET", "/healthz", nil))
		if recorder.Code != 200 || recorder.Body.String() != "{\"status\":\"ok\"}\n" {
			t.Fatalf("learner=%v: %d %s", learner, recorder.Code, recorder.Body.String())
		}
	}
}
