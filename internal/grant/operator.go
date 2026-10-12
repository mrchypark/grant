package grant

import (
	"fmt"
	"net/http"
)

// OperatorHandler is only the native recovery and membership surface.
func (s *Store) OperatorHandler() (http.Handler, error) {
	if s.db == nil {
		return nil, fmt.Errorf("Rhiza voter required for operator listener")
	}
	return s.db.OperatorHandler(), nil
}
