package grant

import "context"

func (s *Service) HttpCapabilityGenerationAdmin(ctx context.Context, provider string, generation uint64) (AdminHttpCapabilityGeneration, error) {
	return s.store.HttpCapabilityGenerationAdmin(ctx, provider, generation)
}
