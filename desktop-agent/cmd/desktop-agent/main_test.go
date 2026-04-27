package main

import (
	"path/filepath"
	"testing"
)

func TestLocalExecutorCandidatesPreferTargetSpecificWorker(t *testing.T) {
	dir := filepath.Join("tmp", "dist")

	got := localExecutorCandidates(dir, "darwin", "arm64")
	wantFirst := filepath.Join(dir, "repass-local-worker-darwin-arm64")
	if got[0] != wantFirst {
		t.Fatalf("first candidate = %q, want %q", got[0], wantFirst)
	}
	if got[1] != filepath.Join(dir, "repass-local-worker") {
		t.Fatalf("generic fallback = %q", got[1])
	}
}

func TestLocalExecutorCandidatesUseWindowsSuffix(t *testing.T) {
	dir := filepath.Join("tmp", "dist")

	got := localExecutorCandidates(dir, "windows", "amd64")
	wantFirst := filepath.Join(dir, "repass-local-worker-windows-amd64.exe")
	if got[0] != wantFirst {
		t.Fatalf("first candidate = %q, want %q", got[0], wantFirst)
	}
	if got[1] != filepath.Join(dir, "repass-local-worker.exe") {
		t.Fatalf("generic fallback = %q", got[1])
	}
}
