package main

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"strconv"
	"strings"
	"time"
)

const (
	stateVersion           = 1
	phasePending           = "pending"
	phaseComplete          = "completed"
	maxStaleStateTempScan  = 256
	maxStaleStateTempClean = 32
)

var recoveryIDPattern = regexp.MustCompile(`^[a-z0-9](?:[a-z0-9-]{0,30}[a-z0-9])?$`)
var restoreSpecPattern = regexp.MustCompile(`^s3:restore/rauthy/([a-z0-9](?:[a-z0-9-]{0,30}[a-z0-9])?)/[a-f0-9]{64}\.enc$`)

type config struct {
	command              []string
	stateFile            string
	restoreIDFile        string
	lifecycleIDFile      string
	lifecycleModeFile    string
	restoreSpecFile      string
	s3KeyFile            string
	s3SecretFile         string
	healthURL            string
	healthInterval       time.Duration
	healthTimeout        time.Duration
	healthRequestTimeout time.Duration
	healthSuccesses      int
	shutdownGrace        time.Duration
	baseEnv              []string
	httpClient           *http.Client
}

type authorization struct {
	recoveryID string
	spec       string
	s3Key      string
	s3Secret   string
}

type lifecycleState struct {
	mode       string
	recoveryID string
}

type restoreState struct {
	Version    int    `json:"version"`
	Phase      string `json:"phase"`
	RecoveryID string `json:"recoveryId"`
	SpecSHA256 string `json:"specSha256"`
}

type childExitError struct {
	err  error
	code int
}

func (e childExitError) Error() string { return fmt.Sprintf("Rauthy exited: %v", e.err) }
func (e childExitError) Unwrap() error { return e.err }
func (e childExitError) ExitCode() int { return e.code }

func configFromEnv(args []string) (config, error) {
	if len(args) == 0 {
		args = []string{"/app/rauthy"}
	}
	interval, err := durationEnv("RAUTHY_LAUNCHER_HEALTH_INTERVAL", time.Second)
	if err != nil {
		return config{}, err
	}
	timeout, err := durationEnv("RAUTHY_LAUNCHER_HEALTH_TIMEOUT", 5*time.Minute)
	if err != nil {
		return config{}, err
	}
	requestTimeout, err := durationEnv("RAUTHY_LAUNCHER_HEALTH_REQUEST_TIMEOUT", 2*time.Second)
	if err != nil {
		return config{}, err
	}
	shutdownGrace, err := durationEnv("RAUTHY_LAUNCHER_SHUTDOWN_GRACE", 15*time.Second)
	if err != nil {
		return config{}, err
	}
	healthSuccesses, err := positiveIntEnv("RAUTHY_LAUNCHER_HEALTH_SUCCESSES", 3)
	if err != nil {
		return config{}, err
	}
	if interval <= 0 || timeout <= 0 || requestTimeout <= 0 || shutdownGrace <= 0 {
		return config{}, errors.New("launcher durations must be positive")
	}

	return config{
		command:              args,
		stateFile:            envOr("RAUTHY_LAUNCHER_STATE_FILE", "/app/data/.rauthy-restore-state.json"),
		restoreIDFile:        envOr("RAUTHY_LAUNCHER_RESTORE_ID_FILE", "/restore/recovery-id"),
		lifecycleIDFile:      envOr("RAUTHY_LAUNCHER_RECOVERY_ID_FILE", "/lifecycle/recovery-id"),
		lifecycleModeFile:    envOr("RAUTHY_LAUNCHER_MODE_FILE", "/lifecycle/mode"),
		restoreSpecFile:      envOr("RAUTHY_LAUNCHER_RESTORE_SPEC_FILE", "/restore/HQL_BACKUP_RESTORE"),
		s3KeyFile:            envOr("RAUTHY_LAUNCHER_S3_KEY_FILE", "/restore/HQL_S3_KEY"),
		s3SecretFile:         envOr("RAUTHY_LAUNCHER_S3_SECRET_FILE", "/restore/HQL_S3_SECRET"),
		healthURL:            envOr("RAUTHY_LAUNCHER_HEALTH_URL", "http://127.0.0.1:8200/ready"),
		healthInterval:       interval,
		healthTimeout:        timeout,
		healthRequestTimeout: requestTimeout,
		healthSuccesses:      healthSuccesses,
		shutdownGrace:        shutdownGrace,
		baseEnv:              os.Environ(),
		httpClient: &http.Client{
			Timeout: requestTimeout,
			CheckRedirect: func(_ *http.Request, _ []*http.Request) error {
				return http.ErrUseLastResponse
			},
		},
	}, nil
}

func positiveIntEnv(name string, fallback int) (int, error) {
	value := os.Getenv(name)
	if value == "" {
		return fallback, nil
	}
	parsed, err := strconv.Atoi(value)
	if err != nil || parsed < 1 || parsed > 20 {
		return 0, fmt.Errorf("%s must be an integer from 1 through 20", name)
	}
	return parsed, nil
}

func durationEnv(name string, fallback time.Duration) (time.Duration, error) {
	value := os.Getenv(name)
	if value == "" {
		return fallback, nil
	}
	d, err := time.ParseDuration(value)
	if err != nil {
		return 0, fmt.Errorf("parse %s: %w", name, err)
	}
	return d, nil
}

func envOr(name, fallback string) string {
	if value := os.Getenv(name); value != "" {
		return value
	}
	return fallback
}

func run(cfg config, signals <-chan os.Signal) error {
	if len(cfg.command) == 0 || cfg.command[0] == "" {
		return errors.New("Rauthy executable is required")
	}

	lifecycle, err := readLifecycle(cfg)
	if err != nil {
		return err
	}
	auth, mounted, err := readAuthorization(cfg, lifecycle)
	if err != nil {
		return err
	}
	state, hasState, err := readState(cfg.stateFile)
	if err != nil {
		return err
	}

	restore := false
	if hasState {
		switch state.Phase {
		case phasePending:
			return fmt.Errorf("recovery %q is pending; refusing automatic replay", state.RecoveryID)
		case phaseComplete:
			if lifecycle.mode == "restore" && state.RecoveryID != lifecycle.recoveryID {
				return errors.New("completed recovery state does not match lifecycle recovery ID")
			}
			if mounted && !state.matches(auth) {
				return errors.New("restore authorization does not match completed recovery state")
			}
		default:
			return fmt.Errorf("unknown recovery phase %q", state.Phase)
		}
	} else if mounted {
		state = newState(phasePending, auth)
		if err := writeState(cfg.stateFile, state); err != nil {
			return fmt.Errorf("persist pending recovery state: %w", err)
		}
		restore = true
	} else if lifecycle.mode == "restore" {
		return errors.New("restore mode requires one-shot authorization or matching completed state")
	}

	childEnv := childEnvironment(cfg.baseEnv, restore)
	if restore {
		childEnv = append(childEnv,
			"HQL_BACKUP_RESTORE="+auth.spec,
			"HQL_S3_KEY="+auth.s3Key,
			"HQL_S3_SECRET="+auth.s3Secret,
		)
	}

	cmd := exec.Command(cfg.command[0], cfg.command[1:]...)
	cmd.Env = childEnv
	cmd.Stdin = os.Stdin
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	if err := cmd.Start(); err != nil {
		return fmt.Errorf("start Rauthy: %w", err)
	}

	exited := make(chan error, 1)
	go func() { exited <- cmd.Wait() }()

	if restore {
		if err := awaitHealthy(cfg, cmd, exited, signals); err != nil {
			return err
		}
		if err := childExitedBeforeCompletion(exited); err != nil {
			return err
		}
		state.Phase = phaseComplete
		if err := writeState(cfg.stateFile, state); err != nil {
			stopChild(cmd, exited, cfg.shutdownGrace)
			return fmt.Errorf("persist completed recovery state: %w", err)
		}
		fmt.Printf("rauthy-launcher: recovery %s completed after health verification\n", auth.recoveryID)
	}

	return supervise(cmd, exited, signals, cfg.shutdownGrace)
}

func readLifecycle(cfg config) (lifecycleState, error) {
	mode, err := readValue(cfg.lifecycleModeFile, 32)
	if err != nil {
		return lifecycleState{}, err
	}
	switch mode {
	case "fresh", "steady", "restore":
	default:
		return lifecycleState{}, fmt.Errorf("invalid lifecycle mode %q", mode)
	}
	recoveryID, err := readValue(cfg.lifecycleIDFile, 32)
	if err != nil {
		return lifecycleState{}, err
	}
	if !recoveryIDPattern.MatchString(recoveryID) {
		return lifecycleState{}, errors.New("lifecycle recovery ID must be a lowercase DNS label of at most 32 characters")
	}
	return lifecycleState{mode: mode, recoveryID: recoveryID}, nil
}

func readAuthorization(cfg config, lifecycle lifecycleState) (authorization, bool, error) {
	// Lifecycle files are always mounted. Only the one-shot Secret files decide
	// whether a restore authorization is present.
	paths := []string{cfg.restoreIDFile, cfg.restoreSpecFile, cfg.s3KeyFile, cfg.s3SecretFile}
	present := 0
	for _, path := range paths {
		info, err := os.Stat(path)
		switch {
		case err == nil && info.Mode().IsRegular():
			present++
		case err == nil:
			return authorization{}, false, fmt.Errorf("restore input %s is not a regular file", path)
		case errors.Is(err, os.ErrNotExist):
		default:
			return authorization{}, false, fmt.Errorf("inspect restore input %s: %w", path, err)
		}
	}
	if present == 0 {
		return authorization{}, false, nil
	}
	if present != len(paths) {
		return authorization{}, false, errors.New("incomplete restore authorization mount")
	}

	restoreID, err := readValue(cfg.restoreIDFile, 128)
	if err != nil {
		return authorization{}, false, err
	}
	spec, err := readValue(cfg.restoreSpecFile, 4096)
	if err != nil {
		return authorization{}, false, err
	}
	s3Key, err := readValue(cfg.s3KeyFile, 4096)
	if err != nil {
		return authorization{}, false, err
	}
	s3Secret, err := readValue(cfg.s3SecretFile, 4096)
	if err != nil {
		return authorization{}, false, err
	}

	if !recoveryIDPattern.MatchString(restoreID) || restoreID != lifecycle.recoveryID {
		return authorization{}, false, errors.New("restore authorization does not match a valid lifecycle recovery ID")
	}
	if lifecycle.mode != "restore" {
		return authorization{}, false, errors.New("restore authorization requires lifecycle restore mode")
	}
	match := restoreSpecPattern.FindStringSubmatch(spec)
	if len(match) != 2 || match[1] != restoreID {
		return authorization{}, false, errors.New("restore directive is not the immutable Rauthy object for this recovery ID")
	}

	return authorization{recoveryID: restoreID, spec: spec, s3Key: s3Key, s3Secret: s3Secret}, true, nil
}

func readValue(path string, limit int64) (string, error) {
	info, err := os.Stat(path)
	if err != nil {
		return "", fmt.Errorf("inspect restore input %s: %w", path, err)
	}
	if !info.Mode().IsRegular() {
		return "", fmt.Errorf("restore input %s is not a regular file", path)
	}
	file, err := os.Open(path)
	if err != nil {
		return "", fmt.Errorf("open restore input %s: %w", path, err)
	}
	defer file.Close()
	value, err := io.ReadAll(io.LimitReader(file, limit+3))
	if err != nil {
		return "", fmt.Errorf("read restore input %s: %w", path, err)
	}
	trimmed := string(value)
	switch {
	case strings.HasSuffix(trimmed, "\r\n"):
		trimmed = strings.TrimSuffix(trimmed, "\r\n")
	case strings.HasSuffix(trimmed, "\n"):
		trimmed = strings.TrimSuffix(trimmed, "\n")
	}
	if int64(len(trimmed)) > limit {
		return "", fmt.Errorf("restore input %s exceeds %d bytes", path, limit)
	}
	if trimmed == "" || strings.IndexByte(trimmed, 0) >= 0 || strings.ContainsAny(trimmed, "\r\n") {
		return "", fmt.Errorf("restore input %s is empty or contains line breaks", path)
	}
	return trimmed, nil
}

func readState(path string) (restoreState, bool, error) {
	if err := cleanupStaleStateTemps(path); err != nil {
		return restoreState{}, false, err
	}
	info, err := os.Lstat(path)
	if errors.Is(err, os.ErrNotExist) {
		return restoreState{}, false, nil
	}
	if err != nil {
		return restoreState{}, false, fmt.Errorf("inspect recovery state: %w", err)
	}
	if !info.Mode().IsRegular() || info.Mode()&os.ModeSymlink != 0 {
		return restoreState{}, false, errors.New("recovery state is not a regular file")
	}
	data, err := os.ReadFile(path)
	if err != nil {
		return restoreState{}, false, fmt.Errorf("read recovery state: %w", err)
	}
	var state restoreState
	if err := json.Unmarshal(data, &state); err != nil {
		return restoreState{}, false, fmt.Errorf("decode recovery state: %w", err)
	}
	if state.Version != stateVersion || !recoveryIDPattern.MatchString(state.RecoveryID) || len(state.SpecSHA256) != 64 {
		return restoreState{}, false, errors.New("recovery state is invalid")
	}
	if _, err := hex.DecodeString(state.SpecSHA256); err != nil {
		return restoreState{}, false, errors.New("recovery state checksum is invalid")
	}
	return state, true, nil
}

func writeState(path string, state restoreState) error {
	parent := filepath.Dir(path)
	if err := os.MkdirAll(parent, 0o700); err != nil {
		return fmt.Errorf("create state directory: %w", err)
	}
	if err := cleanupStaleStateTemps(path); err != nil {
		return err
	}
	data, err := json.Marshal(state)
	if err != nil {
		return fmt.Errorf("encode state: %w", err)
	}
	data = append(data, '\n')
	tmp, err := os.CreateTemp(parent, stateTempPrefix(path)+"*")
	if err != nil {
		return fmt.Errorf("create temporary state: %w", err)
	}
	tmpPath := tmp.Name()
	ok := false
	defer func() {
		if !ok {
			_ = os.Remove(tmpPath)
		}
	}()
	if err := tmp.Chmod(0o600); err != nil {
		_ = tmp.Close()
		return fmt.Errorf("secure temporary state: %w", err)
	}
	if _, err := tmp.Write(data); err != nil {
		_ = tmp.Close()
		return fmt.Errorf("write temporary state: %w", err)
	}
	if err := tmp.Sync(); err != nil {
		_ = tmp.Close()
		return fmt.Errorf("sync temporary state: %w", err)
	}
	if err := tmp.Close(); err != nil {
		return fmt.Errorf("close temporary state: %w", err)
	}
	if err := os.Rename(tmpPath, path); err != nil {
		return fmt.Errorf("replace recovery state: %w", err)
	}
	dir, err := os.Open(parent)
	if err != nil {
		return fmt.Errorf("open state directory: %w", err)
	}
	defer dir.Close()
	if err := dir.Sync(); err != nil {
		return fmt.Errorf("sync state directory: %w", err)
	}
	ok = true
	return nil
}

func stateTempPrefix(path string) string {
	return filepath.Base(path) + ".tmp-"
}

func isStateTempName(path, name string) bool {
	suffix, found := strings.CutPrefix(name, stateTempPrefix(path))
	if !found || len(suffix) == 0 || len(suffix) > 10 {
		return false
	}
	for _, character := range suffix {
		if character < '0' || character > '9' {
			return false
		}
	}
	return true
}

func cleanupStaleStateTemps(path string) error {
	parent := filepath.Dir(path)
	directory, err := os.Open(parent)
	if errors.Is(err, os.ErrNotExist) {
		return nil
	}
	if err != nil {
		return fmt.Errorf("open state directory for temporary cleanup: %w", err)
	}
	defer directory.Close()
	entries, readErr := directory.ReadDir(maxStaleStateTempScan)
	if readErr != nil && !errors.Is(readErr, io.EOF) {
		return fmt.Errorf("scan temporary recovery states: %w", readErr)
	}
	removed := 0
	for _, entry := range entries {
		if removed >= maxStaleStateTempClean || !isStateTempName(path, entry.Name()) {
			continue
		}
		temporaryPath := filepath.Join(parent, entry.Name())
		info, err := os.Lstat(temporaryPath)
		if errors.Is(err, os.ErrNotExist) {
			continue
		}
		if err != nil {
			return fmt.Errorf("inspect stale temporary state: %w", err)
		}
		if !info.Mode().IsRegular() || info.Mode()&os.ModeSymlink != 0 || info.Mode().Perm() != 0o600 {
			continue
		}
		if err := os.Remove(temporaryPath); err != nil && !errors.Is(err, os.ErrNotExist) {
			return fmt.Errorf("remove stale temporary state: %w", err)
		}
		removed++
	}
	return nil
}

func newState(phase string, auth authorization) restoreState {
	sum := sha256.Sum256([]byte(auth.spec))
	return restoreState{Version: stateVersion, Phase: phase, RecoveryID: auth.recoveryID, SpecSHA256: hex.EncodeToString(sum[:])}
}

func (s restoreState) matches(auth authorization) bool {
	want := newState(s.Phase, auth)
	return s.RecoveryID == want.RecoveryID && s.SpecSHA256 == want.SpecSHA256
}

func childEnvironment(env []string, restore bool) []string {
	filtered := make([]string, 0, len(env))
	for _, item := range env {
		name, _, _ := strings.Cut(item, "=")
		switch name {
		case "HQL_BACKUP_RESTORE":
			continue
		case "HQL_S3_KEY", "HQL_S3_SECRET":
			if restore {
				continue
			}
			filtered = append(filtered, item)
		default:
			filtered = append(filtered, item)
		}
	}
	return filtered
}

func awaitHealthy(cfg config, cmd *exec.Cmd, exited <-chan error, signals <-chan os.Signal) error {
	deadline := time.NewTimer(cfg.healthTimeout)
	defer deadline.Stop()
	ticker := time.NewTicker(cfg.healthInterval)
	defer ticker.Stop()

	consecutive := 0
	for {
		if err := childExitedBeforeCompletion(exited); err != nil {
			return err
		}
		if healthy(cfg) {
			consecutive++
		} else {
			consecutive = 0
		}
		if err := childExitedBeforeCompletion(exited); err != nil {
			return err
		}
		if consecutive >= cfg.healthSuccesses {
			return nil
		}
		select {
		case err := <-exited:
			return childError(err, "before restore health verification")
		case sig := <-signals:
			if err := forwardSignalAndWait(cmd, exited, sig, cfg.shutdownGrace); err != nil {
				return fmt.Errorf("received %s while recovery was pending: %w", sig, err)
			}
			return fmt.Errorf("received %s while recovery was pending; recovery remains pending", sig)
		case <-deadline.C:
			stopChild(cmd, exited, cfg.shutdownGrace)
			return errors.New("timed out waiting for Rauthy health; recovery remains pending")
		case <-ticker.C:
		}
	}
}

func healthy(cfg config) bool {
	request, err := http.NewRequest(http.MethodGet, cfg.healthURL, nil)
	if err != nil {
		return false
	}
	response, err := cfg.httpClient.Do(request)
	if err != nil {
		return false
	}
	defer response.Body.Close()
	_, _ = io.Copy(io.Discard, io.LimitReader(response.Body, 4096))
	return response.StatusCode >= 200 && response.StatusCode < 300
}

func supervise(cmd *exec.Cmd, exited <-chan error, signals <-chan os.Signal, grace time.Duration) error {
	for {
		select {
		case err := <-exited:
			return childError(err, "")
		case sig := <-signals:
			return forwardSignalAndWait(cmd, exited, sig, grace)
		}
	}
}

func childExitedBeforeCompletion(exited <-chan error) error {
	select {
	case err := <-exited:
		return childError(err, "before restore health verification completed")
	default:
		return nil
	}
}

func forwardSignalAndWait(cmd *exec.Cmd, exited <-chan error, sig os.Signal, grace time.Duration) error {
	if err := cmd.Process.Signal(sig); err != nil && !errors.Is(err, os.ErrProcessDone) {
		return fmt.Errorf("forward signal to Rauthy: %w", err)
	}
	timer := time.NewTimer(grace)
	defer timer.Stop()
	select {
	case err := <-exited:
		return childError(err, "")
	case <-timer.C:
		if err := cmd.Process.Kill(); err != nil && !errors.Is(err, os.ErrProcessDone) {
			return fmt.Errorf("kill Rauthy after signal grace timeout: %w", err)
		}
		<-exited
		return fmt.Errorf("Rauthy did not exit within %s after %s and was killed", grace, sig)
	}
}

func stopChild(cmd *exec.Cmd, exited <-chan error, grace time.Duration) {
	_ = cmd.Process.Signal(os.Interrupt)
	timer := time.NewTimer(grace)
	defer timer.Stop()
	select {
	case <-exited:
		return
	case <-timer.C:
		_ = cmd.Process.Kill()
		<-exited
	}
}

func verifyExecutableContract(path string, args []string, timeout time.Duration) error {
	info, err := os.Stat(path)
	if err != nil {
		return fmt.Errorf("inspect Rauthy executable: %w", err)
	}
	if !info.Mode().IsRegular() || info.Mode().Perm()&0o111 == 0 {
		return errors.New("Rauthy executable is not a regular executable file")
	}
	ctx, cancel := context.WithTimeout(context.Background(), timeout)
	defer cancel()
	cmd := exec.CommandContext(ctx, path, args...)
	cmd.Env = childEnvironment(os.Environ(), false)
	cmd.Stdout = io.Discard
	cmd.Stderr = io.Discard
	if err := cmd.Run(); err != nil {
		if errors.Is(ctx.Err(), context.DeadlineExceeded) {
			return errors.New("Rauthy executable contract timed out")
		}
		return fmt.Errorf("Rauthy executable contract failed: %w", err)
	}
	return nil
}

func childError(err error, context string) error {
	if err == nil {
		if context == "" {
			return nil
		}
		return fmt.Errorf("Rauthy exited successfully %s", context)
	}
	code := 1
	var exitErr *exec.ExitError
	if errors.As(err, &exitErr) {
		code = exitErr.ExitCode()
	}
	if context != "" {
		return childExitError{err: fmt.Errorf("%w %s", err, context), code: code}
	}
	return childExitError{err: err, code: code}
}
