package api

import (
	"strconv"
	"strings"
)

func (s *Server) httpCapabilityGeneration(r *request) (any, error) {
	raw := r.PathValue("generation")
	generation, err := strconv.ParseUint(raw, 10, 63)
	if err != nil || generation == 0 || strings.ContainsFunc(raw, func(c rune) bool { return c < '0' || c > '9' }) {
		return nil, failure(400, "generation must be a positive 64-bit database integer")
	}
	return s.service.HttpCapabilityGenerationAdmin(r.Context(), r.PathValue("providerId"), generation)
}
