package main

import (
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"os"
	"os/exec"
	"os/signal"
	"path/filepath"
	"reflect"
	"strings"
	"sync/atomic"
	"syscall"
	"testing"
	"time"
)

const testRecoveryID = "recovery-test-1"
const testRestoreSpec = "s3:restore/rauthy/recovery-test-1/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.enc"

func TestRestoreCompletesOnlyAfterHealthAndSuppressesReplay(t *testing.T) {
	tmp := t.TempDir()
	var healthHits atomic.Int32
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		hit := healthHits.Add(1)
		if hit == 2 {
			http.Error(w, "not stable", http.StatusServiceUnavailable)
			return
		}
		w.WriteHeader(http.StatusNoContent)
	}))
	defer server.Close()

	writeAuthorization(t, tmp, testRecoveryID, testRestoreSpec)
	firstCapture := filepath.Join(tmp, "first.json")
	cfg := testConfig(t, tmp, server.URL, firstCapture)
	cfg.command = helperCommand(t, "capture-delay", firstCapture)
	if err := run(cfg, make(chan os.Signal)); err != nil {
		t.Fatalf("first restore run: %v", err)
	}
	assertCapturedEnv(t, firstCapture, true)

	state, present, err := readState(cfg.stateFile)
	if err != nil || !present {
		t.Fatalf("read completed state: present=%v err=%v", present, err)
	}
	if state.Phase != phaseComplete || state.RecoveryID != testRecoveryID {
		t.Fatalf("unexpected completed state: %+v", state)
	}
	if healthHits.Load() < 5 {
		t.Fatalf("completion used fewer than three consecutive healthy checks: %d", healthHits.Load())
	}

	if err := os.RemoveAll(filepath.Join(tmp, "restore")); err != nil {
		t.Fatal(err)
	}
	secondCapture := filepath.Join(tmp, "second.json")
	cfg.command[len(cfg.command)-1] = secondCapture
	if err := run(cfg, make(chan os.Signal)); err != nil {
		t.Fatalf("replay-suppressed run: %v", err)
	}
	assertCapturedEnv(t, secondCapture, false)
}

func TestRestoreModeWithoutAuthorizationOrCompletedStateFailsClosed(t *testing.T) {
	tmp := t.TempDir()
	writeFile(t, filepath.Join(tmp, "lifecycle", "recovery-id"), testRecoveryID)
	writeFile(t, filepath.Join(tmp, "lifecycle", "mode"), "restore")
	cfg := testConfig(t, tmp, "http://127.0.0.1:1/unused", filepath.Join(tmp, "capture.json"))
	if err := run(cfg, make(chan os.Signal)); err == nil || !strings.Contains(err.Error(), "requires one-shot authorization") {
		t.Fatalf("restore without authorization error = %v", err)
	}
}

func TestHealthyResponseDoesNotCompleteAfterChildExit(t *testing.T) {
	tmp := t.TempDir()
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusNoContent)
	}))
	defer server.Close()
	writeAuthorization(t, tmp, testRecoveryID, testRestoreSpec)
	cfg := testConfig(t, tmp, server.URL, filepath.Join(tmp, "capture.json"))
	falseExecutable, err := exec.LookPath("false")
	if err != nil {
		t.Fatal(err)
	}
	cfg.command = []string{falseExecutable}
	if err := run(cfg, make(chan os.Signal)); err == nil || !strings.Contains(err.Error(), "before restore health verification") {
		t.Fatalf("exited child error = %v", err)
	}
	state, present, err := readState(cfg.stateFile)
	if err != nil || !present || state.Phase != phasePending {
		t.Fatalf("state after child exit = %+v, present=%v, err=%v", state, present, err)
	}
}

func TestPendingRecoveryFailsClosedAfterChildExit(t *testing.T) {
	tmp := t.TempDir()
	writeAuthorization(t, tmp, testRecoveryID, testRestoreSpec)
	cfg := testConfig(t, tmp, "http://127.0.0.1:1/unhealthy", filepath.Join(tmp, "first.json"))
	err := run(cfg, make(chan os.Signal))
	if err == nil || !strings.Contains(err.Error(), "before restore health verification") {
		t.Fatalf("first run error = %v", err)
	}
	state, present, stateErr := readState(cfg.stateFile)
	if stateErr != nil || !present || state.Phase != phasePending {
		t.Fatalf("pending state: %+v present=%v err=%v", state, present, stateErr)
	}

	secondCapture := filepath.Join(tmp, "second.json")
	cfg.command[len(cfg.command)-1] = secondCapture
	err = run(cfg, make(chan os.Signal))
	if err == nil || !strings.Contains(err.Error(), "refusing automatic replay") {
		t.Fatalf("restart error = %v", err)
	}
	if _, statErr := os.Stat(secondCapture); !errors.Is(statErr, os.ErrNotExist) {
		t.Fatalf("child unexpectedly started on pending recovery: %v", statErr)
	}
}

func TestNormalStartupWithoutAuthorization(t *testing.T) {
	tmp := t.TempDir()
	writeFile(t, filepath.Join(tmp, "lifecycle", "recovery-id"), "none")
	writeFile(t, filepath.Join(tmp, "lifecycle", "mode"), "fresh")
	capture := filepath.Join(tmp, "normal.json")
	cfg := testConfig(t, tmp, "http://127.0.0.1:1/unused", capture)
	if err := run(cfg, make(chan os.Signal)); err != nil {
		t.Fatalf("normal run: %v", err)
	}
	assertCapturedEnv(t, capture, false)
	if _, err := os.Stat(cfg.stateFile); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("normal startup created recovery state: %v", err)
	}
}

func TestLifecycleValueContract(t *testing.T) {
	t.Run("accepts LF", func(t *testing.T) {
		tmp := t.TempDir()
		writeFile(t, filepath.Join(tmp, "lifecycle", "mode"), "restore\n")
		writeFile(t, filepath.Join(tmp, "lifecycle", "recovery-id"), testRecoveryID+"\n")
		cfg := testConfig(t, tmp, "http://127.0.0.1:1/unused", filepath.Join(tmp, "capture.json"))
		state, err := readLifecycle(cfg)
		if err != nil || state.mode != "restore" || state.recoveryID != testRecoveryID {
			t.Fatalf("LF lifecycle state = %+v, err = %v", state, err)
		}
	})

	t.Run("accepts CRLF", func(t *testing.T) {
		tmp := t.TempDir()
		writeFile(t, filepath.Join(tmp, "lifecycle", "mode"), "restore\r\n")
		writeFile(t, filepath.Join(tmp, "lifecycle", "recovery-id"), testRecoveryID+"\r\n")
		cfg := testConfig(t, tmp, "http://127.0.0.1:1/unused", filepath.Join(tmp, "capture.json"))
		state, err := readLifecycle(cfg)
		if err != nil || state.mode != "restore" || state.recoveryID != testRecoveryID {
			t.Fatalf("CRLF lifecycle state = %+v, err = %v", state, err)
		}
	})

	t.Run("accepts maximum recovery ID with CRLF", func(t *testing.T) {
		tmp := t.TempDir()
		maximumID := strings.Repeat("a", 32)
		writeFile(t, filepath.Join(tmp, "lifecycle", "mode"), "restore\r\n")
		writeFile(t, filepath.Join(tmp, "lifecycle", "recovery-id"), maximumID+"\r\n")
		cfg := testConfig(t, tmp, "http://127.0.0.1:1/unused", filepath.Join(tmp, "capture.json"))
		state, err := readLifecycle(cfg)
		if err != nil || state.recoveryID != maximumID {
			t.Fatalf("maximum CRLF lifecycle state = %+v, err = %v", state, err)
		}
	})

	invalidValues := []struct {
		name  string
		value string
	}{
		{name: "leading space", value: " restore\n"},
		{name: "trailing space", value: "restore \n"},
		{name: "internal space", value: "rest ore\n"},
		{name: "tab", value: "restore\t\n"},
		{name: "extra line", value: "restore\nsteady\n"},
		{name: "NUL", value: "restore\x00\n"},
		{name: "empty", value: ""},
		{name: "bare CR", value: "restore\r"},
	}
	for _, tt := range invalidValues {
		t.Run("mode rejects "+tt.name, func(t *testing.T) {
			tmp := t.TempDir()
			writeFile(t, filepath.Join(tmp, "lifecycle", "mode"), tt.value)
			writeFile(t, filepath.Join(tmp, "lifecycle", "recovery-id"), testRecoveryID+"\n")
			cfg := testConfig(t, tmp, "http://127.0.0.1:1/unused", filepath.Join(tmp, "capture.json"))
			if _, err := readLifecycle(cfg); err == nil {
				t.Fatal("invalid lifecycle mode was accepted")
			}
		})
	}

	invalidIDs := []struct {
		name  string
		value string
	}{
		{name: "leading space", value: " " + testRecoveryID + "\n"},
		{name: "trailing space", value: testRecoveryID + " \n"},
		{name: "internal space", value: "recovery test-1\n"},
		{name: "tab", value: "recovery\ttest-1\n"},
		{name: "extra line", value: testRecoveryID + "\nother\n"},
		{name: "NUL", value: "recovery\x00test-1\n"},
		{name: "empty", value: ""},
		{name: "bare CR", value: testRecoveryID + "\r"},
		{name: "33 bytes", value: strings.Repeat("a", 33) + "\n"},
	}
	for _, tt := range invalidIDs {
		t.Run("recovery ID rejects "+tt.name, func(t *testing.T) {
			tmp := t.TempDir()
			writeFile(t, filepath.Join(tmp, "lifecycle", "mode"), "restore\n")
			writeFile(t, filepath.Join(tmp, "lifecycle", "recovery-id"), tt.value)
			cfg := testConfig(t, tmp, "http://127.0.0.1:1/unused", filepath.Join(tmp, "capture.json"))
			if _, err := readLifecycle(cfg); err == nil {
				t.Fatal("invalid lifecycle recovery ID was accepted")
			}
		})
	}
}

func TestReadValueRejectsNonRegularFilesWithoutBlocking(t *testing.T) {
	t.Run("directory", func(t *testing.T) {
		if _, err := readValue(t.TempDir(), 32); err == nil || !strings.Contains(err.Error(), "not a regular file") {
			t.Fatalf("directory error = %v", err)
		}
	})

	t.Run("FIFO", func(t *testing.T) {
		path := filepath.Join(t.TempDir(), "lifecycle-fifo")
		if err := syscall.Mkfifo(path, 0o600); err != nil {
			t.Fatal(err)
		}
		done := make(chan error, 1)
		go func() {
			_, err := readValue(path, 32)
			done <- err
		}()
		select {
		case err := <-done:
			if err == nil || !strings.Contains(err.Error(), "not a regular file") {
				t.Fatalf("FIFO error = %v", err)
			}
		case <-time.After(250 * time.Millisecond):
			t.Fatal("readValue blocked while opening a FIFO")
		}
	})
}

func TestStateTemporaryFilesAreUniqueAndAtomic(t *testing.T) {
	tmp := t.TempDir()
	statePath := filepath.Join(tmp, "state.json")
	first, err := os.CreateTemp(tmp, stateTempPrefix(statePath)+"*")
	if err != nil {
		t.Fatal(err)
	}
	firstName := first.Name()
	if err := first.Close(); err != nil {
		t.Fatal(err)
	}
	second, err := os.CreateTemp(tmp, stateTempPrefix(statePath)+"*")
	if err != nil {
		t.Fatal(err)
	}
	secondName := second.Name()
	if err := second.Close(); err != nil {
		t.Fatal(err)
	}
	if firstName == secondName || !isStateTempName(statePath, filepath.Base(firstName)) || !isStateTempName(statePath, filepath.Base(secondName)) {
		t.Fatalf("temporary state names are not unique launcher names: %q, %q", firstName, secondName)
	}

	state := newState(phasePending, authorization{recoveryID: testRecoveryID, spec: testRestoreSpec})
	if err := writeState(statePath, state); err != nil {
		t.Fatalf("write state: %v", err)
	}
	written, present, err := readState(statePath)
	if err != nil || !present || written != state {
		t.Fatalf("atomic state = %+v, present=%v, err=%v", written, present, err)
	}
	info, err := os.Stat(statePath)
	if err != nil {
		t.Fatal(err)
	}
	if info.Mode().Perm() != 0o600 {
		t.Fatalf("state mode = %o, want 600", info.Mode().Perm())
	}
}

func TestStaleStateTemporaryCleanupIsBoundedAndSafe(t *testing.T) {
	tmp := t.TempDir()
	statePath := filepath.Join(tmp, "state.json")
	staleNames := make([]string, 0, maxStaleStateTempClean+8)
	for range maxStaleStateTempClean + 8 {
		file, err := os.CreateTemp(tmp, stateTempPrefix(statePath)+"*")
		if err != nil {
			t.Fatal(err)
		}
		staleNames = append(staleNames, file.Name())
		if err := file.Close(); err != nil {
			t.Fatal(err)
		}
	}
	unrelated := filepath.Join(tmp, stateTempPrefix(statePath)+"not-launcher-random")
	writeFile(t, unrelated, "unrelated")
	unrelatedNumeric := filepath.Join(tmp, stateTempPrefix(statePath)+"1111111111")
	writeFile(t, unrelatedNumeric, "unrelated numeric file")
	if err := os.Chmod(unrelatedNumeric, 0o644); err != nil {
		t.Fatal(err)
	}
	target := filepath.Join(tmp, "symlink-target")
	writeFile(t, target, "target")
	symlink := filepath.Join(tmp, stateTempPrefix(statePath)+"1234567890")
	if err := os.Symlink(target, symlink); err != nil {
		t.Fatal(err)
	}
	fifo := filepath.Join(tmp, stateTempPrefix(statePath)+"9876543210")
	if err := syscall.Mkfifo(fifo, 0o600); err != nil {
		t.Fatal(err)
	}

	if _, present, err := readState(statePath); err != nil || present {
		t.Fatalf("read missing state after stale cleanup: present=%v err=%v", present, err)
	}
	remaining := 0
	for _, name := range staleNames {
		if _, err := os.Stat(name); err == nil {
			remaining++
		} else if !errors.Is(err, os.ErrNotExist) {
			t.Fatal(err)
		}
	}
	if remaining != 8 {
		t.Fatalf("bounded cleanup left %d stale files, want 8", remaining)
	}
	for _, path := range []string{unrelated, unrelatedNumeric, target, symlink, fifo} {
		if _, err := os.Lstat(path); err != nil {
			t.Fatalf("safe cleanup removed %s: %v", path, err)
		}
	}
}

func TestWriteStateFailureRemovesItsTemporaryFile(t *testing.T) {
	tmp := t.TempDir()
	statePath := filepath.Join(tmp, "state.json")
	if err := os.Mkdir(statePath, 0o700); err != nil {
		t.Fatal(err)
	}
	state := newState(phasePending, authorization{recoveryID: testRecoveryID, spec: testRestoreSpec})
	if err := writeState(statePath, state); err == nil {
		t.Fatal("writing state over a directory unexpectedly succeeded")
	}
	entries, err := os.ReadDir(tmp)
	if err != nil {
		t.Fatal(err)
	}
	for _, entry := range entries {
		if isStateTempName(statePath, entry.Name()) {
			t.Fatalf("failed write left temporary state %s", entry.Name())
		}
	}
}

func TestRejectsIncompleteOrMismatchedAuthorization(t *testing.T) {
	tests := []struct {
		name string
		edit func(t *testing.T, tmp string)
	}{
		{
			name: "missing secret",
			edit: func(t *testing.T, tmp string) {
				t.Helper()
				if err := os.Remove(filepath.Join(tmp, "restore", "HQL_S3_SECRET")); err != nil {
					t.Fatal(err)
				}
			},
		},
		{
			name: "different lifecycle id",
			edit: func(t *testing.T, tmp string) {
				t.Helper()
				writeFile(t, filepath.Join(tmp, "lifecycle", "recovery-id"), "other-recovery")
			},
		},
		{
			name: "recovery ids have divergent suffixes",
			edit: func(t *testing.T, tmp string) {
				t.Helper()
				writeFile(t, filepath.Join(tmp, "restore", "recovery-id"), "recovery-test-restore")
				writeFile(t, filepath.Join(tmp, "lifecycle", "recovery-id"), "recovery-test-lifecycle")
			},
		},
		{
			name: "object belongs to another recovery",
			edit: func(t *testing.T, tmp string) {
				t.Helper()
				writeFile(t, filepath.Join(tmp, "restore", "HQL_BACKUP_RESTORE"), "s3:restore/rauthy/other/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.enc")
			},
		},
		{
			name: "lifecycle not in restore mode",
			edit: func(t *testing.T, tmp string) {
				t.Helper()
				writeFile(t, filepath.Join(tmp, "lifecycle", "mode"), "steady")
			},
		},
		{
			name: "uppercase recovery id",
			edit: func(t *testing.T, tmp string) {
				t.Helper()
				writeFile(t, filepath.Join(tmp, "restore", "recovery-id"), "Recovery-Test")
				writeFile(t, filepath.Join(tmp, "lifecycle", "recovery-id"), "Recovery-Test")
			},
		},
		{
			name: "recovery id too long",
			edit: func(t *testing.T, tmp string) {
				t.Helper()
				longID := strings.Repeat("a", 33)
				writeFile(t, filepath.Join(tmp, "restore", "recovery-id"), longID)
				writeFile(t, filepath.Join(tmp, "lifecycle", "recovery-id"), longID)
			},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			tmp := t.TempDir()
			writeAuthorization(t, tmp, testRecoveryID, testRestoreSpec)
			tt.edit(t, tmp)
			cfg := testConfig(t, tmp, "http://127.0.0.1:1/unused", filepath.Join(tmp, "capture.json"))
			if err := run(cfg, make(chan os.Signal)); err == nil {
				t.Fatal("invalid authorization was accepted")
			}
		})
	}
}

func TestSignalIsForwardedToChild(t *testing.T) {
	tmp := t.TempDir()
	cfg := testConfig(t, tmp, "http://127.0.0.1:1/unused", filepath.Join(tmp, "signal.json"))
	cfg.command = helperCommand(t, "wait-default-signal", filepath.Join(tmp, "signal.txt"))
	signals := make(chan os.Signal, 1)
	done := make(chan error, 1)
	go func() { done <- run(cfg, signals) }()

	waitForFile(t, filepath.Join(tmp, "signal.txt.ready"))
	signals <- syscall.SIGTERM
	select {
	case err := <-done:
		var childErr childExitError
		if !errors.As(err, &childErr) {
			t.Fatalf("run after signal: %v", err)
		}
		var exitErr *exec.ExitError
		if !errors.As(err, &exitErr) || exitErr.ProcessState.Sys().(syscall.WaitStatus).Signal() != syscall.SIGTERM {
			t.Fatalf("child did not exit from forwarded SIGTERM: %v", err)
		}
	case <-time.After(8 * time.Second):
		t.Fatal("launcher did not forward signal")
	}
}

func TestSignalDuringRestoreLeavesPendingState(t *testing.T) {
	tmp := t.TempDir()
	writeAuthorization(t, tmp, testRecoveryID, testRestoreSpec)
	signalFile := filepath.Join(tmp, "restore-signal.txt")
	cfg := testConfig(t, tmp, "http://127.0.0.1:1/unhealthy", signalFile)
	cfg.command = helperCommand(t, "wait-default-signal", signalFile)
	signals := make(chan os.Signal, 1)
	done := make(chan error, 1)
	go func() { done <- run(cfg, signals) }()

	waitForFile(t, signalFile+".ready")
	signals <- syscall.SIGTERM
	select {
	case err := <-done:
		if err == nil || !strings.Contains(err.Error(), "while recovery was pending") {
			t.Fatalf("pending signal error = %v", err)
		}
	case <-time.After(8 * time.Second):
		t.Fatal("launcher did not stop a signaled restore child")
	}
	state, present, err := readState(cfg.stateFile)
	if err != nil || !present || state.Phase != phasePending {
		t.Fatalf("pending state after signal: %+v present=%v err=%v", state, present, err)
	}
}

func TestIgnoredSignalKillsOnlyAfterGraceTimeout(t *testing.T) {
	tmp := t.TempDir()
	output := filepath.Join(tmp, "ignored.txt")
	cfg := testConfig(t, tmp, "http://127.0.0.1:1/unused", output)
	cfg.command = helperCommand(t, "ignore-signal", output)
	cfg.shutdownGrace = 75 * time.Millisecond
	signals := make(chan os.Signal, 1)
	done := make(chan error, 1)
	go func() { done <- run(cfg, signals) }()
	waitForFile(t, output+".ready")
	started := time.Now()
	signals <- syscall.SIGTERM
	err := <-done
	if err == nil || !strings.Contains(err.Error(), "was killed") {
		t.Fatalf("ignored signal error = %v", err)
	}
	if time.Since(started) < cfg.shutdownGrace {
		t.Fatal("child was killed before the configured grace timeout")
	}
}

func TestExecutableContract(t *testing.T) {
	t.Setenv("GO_WANT_RAUTHY_LAUNCHER_HELPER", "1")
	if err := verifyExecutableContract(os.Args[0], []string{"-test.run=TestHelperProcess", "--", "contract", "unused"}, 2*time.Second); err != nil {
		t.Fatalf("valid executable contract: %v", err)
	}
	notExecutable := filepath.Join(t.TempDir(), "rauthy")
	if err := os.WriteFile(notExecutable, []byte("not executable"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := verifyExecutableContract(notExecutable, nil, time.Second); err == nil {
		t.Fatal("non-executable contract target was accepted")
	}
}

func TestHelperProcess(t *testing.T) {
	if os.Getenv("GO_WANT_RAUTHY_LAUNCHER_HELPER") != "1" {
		return
	}
	separator := 0
	for i, arg := range os.Args {
		if arg == "--" {
			separator = i
			break
		}
	}
	mode := os.Args[separator+1]
	output := os.Args[separator+2]
	switch mode {
	case "capture", "capture-delay":
		captured := map[string]string{}
		for _, name := range []string{"HQL_BACKUP_RESTORE", "HQL_S3_KEY", "HQL_S3_SECRET"} {
			if value, ok := os.LookupEnv(name); ok {
				captured[name] = value
			}
		}
		data, _ := json.Marshal(captured)
		_ = os.WriteFile(output, data, 0o600)
		if mode == "capture-delay" {
			time.Sleep(250 * time.Millisecond)
		}
		os.Exit(0)
	case "wait-signal":
		signals := make(chan os.Signal, 1)
		osSignalNotify(signals)
		_ = os.WriteFile(output+".ready", []byte("ready\n"), 0o600)
		<-signals
		_ = os.WriteFile(output, []byte("terminated\n"), 0o600)
		os.Exit(0)
	case "wait-default-signal":
		_ = os.WriteFile(output+".ready", []byte("ready\n"), 0o600)
		for {
			time.Sleep(time.Second)
		}
	case "ignore-signal":
		signal.Ignore(syscall.SIGTERM, syscall.SIGINT)
		_ = os.WriteFile(output+".ready", []byte("ready\n"), 0o600)
		for {
			time.Sleep(time.Second)
		}
	case "contract":
		os.Exit(0)
	default:
		os.Exit(2)
	}
}

func waitForFile(t *testing.T, path string) {
	t.Helper()
	deadline := time.Now().Add(3 * time.Second)
	for time.Now().Before(deadline) {
		if _, err := os.Stat(path); err == nil {
			return
		}
		time.Sleep(10 * time.Millisecond)
	}
	t.Fatalf("timed out waiting for %s", path)
}

func osSignalNotify(ch chan<- os.Signal) {
	signal.Notify(ch, syscall.SIGTERM, syscall.SIGINT)
}

func writeAuthorization(t *testing.T, tmp, recoveryID, spec string) {
	t.Helper()
	writeFile(t, filepath.Join(tmp, "restore", "recovery-id"), recoveryID)
	writeFile(t, filepath.Join(tmp, "lifecycle", "recovery-id"), recoveryID)
	writeFile(t, filepath.Join(tmp, "lifecycle", "mode"), "restore")
	writeFile(t, filepath.Join(tmp, "restore", "HQL_BACKUP_RESTORE"), spec)
	writeFile(t, filepath.Join(tmp, "restore", "HQL_S3_KEY"), "reader-key")
	writeFile(t, filepath.Join(tmp, "restore", "HQL_S3_SECRET"), "reader-secret")
}

func writeFile(t *testing.T, path, value string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte(value), 0o600); err != nil {
		t.Fatal(err)
	}
}

func testConfig(t *testing.T, tmp, healthURL, capture string) config {
	t.Helper()
	if _, err := os.Stat(filepath.Join(tmp, "lifecycle", "mode")); errors.Is(err, os.ErrNotExist) {
		writeFile(t, filepath.Join(tmp, "lifecycle", "mode"), "steady")
	}
	if _, err := os.Stat(filepath.Join(tmp, "lifecycle", "recovery-id")); errors.Is(err, os.ErrNotExist) {
		writeFile(t, filepath.Join(tmp, "lifecycle", "recovery-id"), "none")
	}
	return config{
		command:              helperCommand(t, "capture", capture),
		stateFile:            filepath.Join(tmp, "data", "state.json"),
		restoreIDFile:        filepath.Join(tmp, "restore", "recovery-id"),
		lifecycleIDFile:      filepath.Join(tmp, "lifecycle", "recovery-id"),
		lifecycleModeFile:    filepath.Join(tmp, "lifecycle", "mode"),
		restoreSpecFile:      filepath.Join(tmp, "restore", "HQL_BACKUP_RESTORE"),
		s3KeyFile:            filepath.Join(tmp, "restore", "HQL_S3_KEY"),
		s3SecretFile:         filepath.Join(tmp, "restore", "HQL_S3_SECRET"),
		healthURL:            healthURL,
		healthInterval:       5 * time.Millisecond,
		healthTimeout:        5 * time.Second,
		healthRequestTimeout: 50 * time.Millisecond,
		healthSuccesses:      3,
		shutdownGrace:        5 * time.Second,
		baseEnv:              append(os.Environ(), "GO_WANT_RAUTHY_LAUNCHER_HELPER=1", "HQL_BACKUP_RESTORE=leaked", "HQL_S3_KEY=writer-key", "HQL_S3_SECRET=writer-secret"),
		httpClient:           &http.Client{Timeout: 50 * time.Millisecond},
	}
}

func helperCommand(t *testing.T, mode, output string) []string {
	t.Helper()
	return []string{os.Args[0], "-test.run=TestHelperProcess", "--", mode, output}
}

func assertCapturedEnv(t *testing.T, path string, restored bool) {
	t.Helper()
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	var values map[string]string
	if err := json.Unmarshal(data, &values); err != nil {
		t.Fatal(err)
	}
	if restored {
		want := map[string]string{
			"HQL_BACKUP_RESTORE": testRestoreSpec,
			"HQL_S3_KEY":         "reader-key",
			"HQL_S3_SECRET":      "reader-secret",
		}
		if !reflect.DeepEqual(values, want) {
			t.Fatalf("restore environment = %#v, want %#v", values, want)
		}
	} else {
		want := map[string]string{
			"HQL_S3_KEY":    "writer-key",
			"HQL_S3_SECRET": "writer-secret",
		}
		if !reflect.DeepEqual(values, want) {
			t.Fatalf("normal writer environment = %#v, want %#v", values, want)
		}
	}
}
