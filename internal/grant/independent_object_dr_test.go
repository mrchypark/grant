package grant

import (
	"bytes"
	"context"
	"crypto/rand"
	"crypto/rsa"
	"crypto/sha256"
	"crypto/x509"
	"encoding/hex"
	"encoding/json"
	"encoding/pem"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"os"
	"os/exec"
	"os/signal"
	"path/filepath"
	"strings"
	"syscall"
	"testing"
	"time"

	"cloud.google.com/go/storage"
	"github.com/Azure/azure-sdk-for-go/sdk/storage/azblob"
	"github.com/minio/minio-go/v7"
	"github.com/minio/minio-go/v7/pkg/credentials"
	"google.golang.org/api/option"
)

// Immutable official multi-platform release indexes; Docker selects the host platform.
var independentImages = []struct{ name, image string }{
	{"s3", "floci/floci@sha256:e0b292df4dc9559b7b1bf4f51bdda4bb8e269276236438cd03cae661260ea8a9"},
	// floci-gcp 0.11.0 and floci-az 0.15.0 release indexes, not ARM64 child manifests.
	{"gcs", "floci/floci-gcp@sha256:eae9683d806ceb643bcb36aee97cdb8cad13c3e2959cd053bf40e481994fde12"},
	{"azure", "floci/floci-az@sha256:530635d603223e120adbf9221a97560cffc6991d65fa0c72d0f79d1097974c89"},
}

type independentFixture struct {
	t                          *testing.T
	ctx                        context.Context
	dir, logs, binary, id, cid string
	env                        []string
	base                       string
	main                       *exec.Cmd
	mainPhase                  string
	mainStarted                time.Time
	createAdmitted             bool
	requestSeq                 int
	mainDone                   chan independentExit
	mainExit                   *independentExit
}

type independentExit struct {
	err      error
	observed time.Time
}

func independentToken() string {
	b := make([]byte, 16)
	if _, err := rand.Read(b); err != nil {
		panic(err)
	}
	return hex.EncodeToString(b)
}
func (f *independentFixture) log(name string, data []byte) {
	f.t.Helper()
	if err := os.WriteFile(filepath.Join(f.logs, name), data, 0600); err != nil {
		f.t.Fatal(err)
	}
}
func (f *independentFixture) command(timeout time.Duration, name string, args ...string) ([]byte, error) {
	ctx, cancel := context.WithTimeout(f.ctx, timeout)
	defer cancel()
	c := exec.CommandContext(ctx, name, args...)
	c.Env = f.env
	return c.CombinedOutput()
}
func (f *independentFixture) docker(args ...string) ([]byte, error) {
	ctx, cancel := context.WithTimeout(f.ctx, 60*time.Second)
	defer cancel()
	cmd := exec.CommandContext(ctx, "docker", args...)
	cmd.Env = f.env
	stderr, err := os.OpenFile(filepath.Join(f.logs, "docker.stderr.log"), os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0600)
	if err != nil {
		return nil, err
	}
	defer stderr.Close()
	cmd.Stderr = stderr
	// Docker warnings must never become part of a CID or other parsed stdout.
	return cmd.Output()
}
func (f *independentFixture) requireCommand(name string, timeout time.Duration, binary string, args ...string) {
	f.t.Helper()
	ctx, cancel := context.WithTimeout(f.ctx, timeout)
	defer cancel()
	cmd := exec.CommandContext(ctx, binary, args...)
	cmd.Env = f.env
	stdout, err := os.Create(filepath.Join(f.logs, name+".stdout.log"))
	if err != nil {
		f.t.Fatal(err)
	}
	defer stdout.Close()
	stderr, err := os.Create(filepath.Join(f.logs, name+".stderr.log"))
	if err != nil {
		f.t.Fatal(err)
	}
	defer stderr.Close()
	cmd.Stdout, cmd.Stderr = stdout, stderr
	canceled := make(chan time.Time, 1)
	cmd.Cancel = func() error {
		canceled <- time.Now().UTC()
		return cmd.Process.Kill()
	}
	started := time.Now().UTC()
	err = cmd.Start()
	if err == nil {
		defer func() {
			if cmd.ProcessState == nil {
				_ = cmd.Process.Kill()
				_ = cmd.Wait()
			}
		}()
		if binary == f.binary || binary == os.Args[0] {
			f.recordChild(name, cmd, binary)
		}
		err = cmd.Wait()
	}
	var cancelAt time.Time
	select {
	case cancelAt = <-canceled:
	default:
	}
	f.processReceipt(name, cmd, started, err, map[string]any{"context_error": fmt.Sprint(ctx.Err()), "cancel_observed_utc": cancelAt, "cancel_signal": "SIGKILL on context cancellation only"})
	exit := 0
	if err != nil {
		exit = -1
		if e, ok := err.(*exec.ExitError); ok {
			exit = e.ExitCode()
		}
	}
	f.log(name+"-exit.txt", []byte(fmt.Sprintf("%d\n", exit)))
	if err != nil {
		f.t.Fatalf("%s FAIL: %v; stdout/stderr and process receipt preserved", name, err)
	}
	f.t.Logf("%s PASS", name)
}

// Persist before Wait so a dead controller cannot strand an unrecorded helper.
func (f *independentFixture) recordChild(phase string, cmd *exec.Cmd, binary string) {
	identity := []string{fmt.Sprint(cmd.Process.Pid), binary}
	// A partial receipt fails closed and preserves work if identity collection is interrupted.
	f.log(phase+"-process.receipt", []byte(strings.Join(append(identity, "", strings.Join(cmd.Args, " ")), "\n")+"\n"))
	for _, field := range []string{"lstart=", "args="} {
		out, err := f.command(5*time.Second, "ps", "-p", fmt.Sprint(cmd.Process.Pid), "-o", field)
		if err != nil {
			// Do not abandon the child if identity capture itself fails.
			_ = cmd.Process.Kill()
			_ = cmd.Wait()
			f.processReceipt(phase+"-identity-failure", cmd, time.Now().UTC(), err, map[string]any{"identity_capture_failed": true})
			if cmd == f.main {
				f.main = nil
				f.mainDone = nil
				f.mainExit = nil
			}
			f.t.Fatalf("record child identity before wait: %v", err)
		}
		identity = append(identity, strings.TrimSpace(string(out)))
	}
	f.log(phase+"-process.receipt", []byte(strings.Join(identity, "\n")+"\n"))
}

func (f *independentFixture) processReceipt(name string, cmd *exec.Cmd, started time.Time, err error, extra map[string]any) {
	state := map[string]any{"started_utc": started, "observed_utc": time.Now().UTC(), "error": fmt.Sprint(err), "exit_code": -1, "signal": "", "pid": 0}
	if cmd.Process != nil {
		state["pid"] = cmd.Process.Pid
	}
	if cmd.ProcessState != nil {
		state["observed_exit_utc"] = time.Now().UTC()
		state["exit_code"] = cmd.ProcessState.ExitCode()
		if status, ok := cmd.ProcessState.Sys().(syscall.WaitStatus); ok && status.Signaled() {
			state["signal"] = status.Signal().String()
		}
	}
	for key, value := range extra {
		state[key] = value
	}
	raw, e := json.MarshalIndent(state, "", "  ")
	if e != nil {
		f.t.Fatal(e)
	}
	f.log(name+"-process.json", raw)
}
func (f *independentFixture) request(method, path string, body any, status int) []byte {
	f.t.Helper()
	var data []byte
	if body != nil {
		data, _ = json.Marshal(body)
	}
	f.requestSeq++
	sequence := f.requestSeq
	started := time.Now().UTC()
	actualStatus := 0
	var response []byte
	var requestError string
	defer func() {
		raw, e := json.MarshalIndent(map[string]any{"method": method, "path": path, "request": json.RawMessage(data), "response": string(response), "status": actualStatus, "expected_status": status, "write_ack": method != "GET" && actualStatus == status, "started_utc": started, "observed_utc": time.Now().UTC(), "error": requestError}, "", "  ")
		if e != nil {
			f.t.Fatal(e)
		}
		f.log(fmt.Sprintf("http-%03d-receipt.json", sequence), raw)
	}()
	ctx, cancel := context.WithTimeout(f.ctx, 15*time.Second)
	defer cancel()
	req, err := http.NewRequestWithContext(ctx, method, f.base+path, bytes.NewReader(data))
	if err != nil {
		requestError = err.Error()
		f.t.Fatal(err)
	}
	req.Header.Set("Content-Type", "application/json")
	if method == "PUT" && strings.HasPrefix(path, "/ops/providers/") && strings.Contains(path, "/enforcers/") {
		req.Header.Set("If-None-Match", "*")
	}
	req.Header.Set("X-Grant-Master-Key", f.id+"-master")
	client := &http.Client{Transport: &http.Transport{Proxy: nil}, CheckRedirect: func(_ *http.Request, _ []*http.Request) error { return http.ErrUseLastResponse }}
	defer client.CloseIdleConnections()
	res, err := client.Do(req)
	if err != nil {
		requestError = err.Error()
		f.t.Fatal(err)
	}
	defer res.Body.Close()
	actualStatus = res.StatusCode
	out, err := io.ReadAll(io.LimitReader(res.Body, 4<<20))
	response = out
	if err != nil {
		requestError = err.Error()
		f.t.Fatal(err)
	}
	if res.StatusCode != status {
		f.t.Fatalf("%s %s HTTP %d want %d: %s", method, path, res.StatusCode, status, out)
	}
	return out
}
func independentPort(t *testing.T) string {
	t.Helper()
	l, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer l.Close()
	return l.Addr().String()
}
func (f *independentFixture) start(phase string) {
	f.t.Helper()
	log, err := os.Create(filepath.Join(f.logs, phase+".stdout.log"))
	if err != nil {
		f.t.Fatal(err)
	}
	f.main = exec.Command(f.binary)
	f.mainPhase = phase
	f.mainStarted = time.Now().UTC()
	f.log(phase+"-open.txt", []byte("main startup requested "+time.Now().UTC().Format(time.RFC3339Nano)+"\n"))
	f.main.Env = f.env
	f.main.Stdout = log
	stderr, err := os.Create(filepath.Join(f.logs, phase+".stderr.log"))
	if err != nil {
		log.Close()
		f.main = nil
		f.t.Fatal(err)
	}
	f.main.Stderr = stderr
	if err = f.main.Start(); err != nil {
		log.Close()
		stderr.Close()
		f.processReceipt(phase+"-start-failure", f.main, f.mainStarted, err, nil)
		f.main = nil
		f.t.Fatal(err)
	}
	log.Close()
	stderr.Close()
	f.log(phase+"-pid.txt", []byte(fmt.Sprint(f.main.Process.Pid)))
	f.recordChild(phase, f.main, f.binary)
	f.mainExit = nil
	f.mainDone = make(chan independentExit, 1)
	cmd, done := f.main, f.mainDone
	go func() { done <- independentExit{err: cmd.Wait(), observed: time.Now().UTC()} }()
	ctx, cancel := context.WithTimeout(f.ctx, 45*time.Second)
	defer cancel()
	client := &http.Client{Timeout: time.Second, Transport: &http.Transport{Proxy: nil}}
	defer client.CloseIdleConnections()
	for {
		if f.pollMainExit() {
			f.t.Fatalf("%s exited before readiness: %v", phase, f.mainExit.err)
		}
		req, _ := http.NewRequestWithContext(ctx, "GET", f.base+"/readyz", nil)
		res, e := client.Do(req)
		if e == nil {
			res.Body.Close()
			if res.StatusCode == 200 {
				f.log(phase+"-ready.txt", []byte("HTTP readiness 200 "+time.Now().UTC().Format(time.RFC3339Nano)+"\n"))
				return
			}
		}
		select {
		case <-ctx.Done():
			f.t.Errorf("%s readiness contract FAILED at 45s: %v; continuing diagnostic observation only", phase, ctx.Err())
			f.observeRecovery(phase+"-readiness-failed", nil)
			f.t.Fatalf("%s readiness: %v", phase, ctx.Err())
		case <-time.After(200 * time.Millisecond):
		}
	}
}

func (f *independentFixture) pollMainExit() bool {
	if f.mainExit != nil {
		return true
	}
	if f.mainDone == nil {
		return false
	}
	select {
	case exit := <-f.mainDone:
		f.mainExit = &exit
		return true
	default:
		return false
	}
}

// Diagnostic coverage is deliberately separate from the 45s readiness contract.
func (f *independentFixture) observeRecovery(phase string, before []byte) {
	started := time.Now().UTC()
	until := started.Add(100 * time.Second)
	diagnosticOK := before != nil
	check := func(path string) (int, []byte, error) {
		ctx, cancel := context.WithTimeout(f.ctx, 3*time.Second)
		defer cancel()
		req, err := http.NewRequestWithContext(ctx, "GET", f.base+path, nil)
		if err != nil {
			return 0, nil, err
		}
		client := &http.Client{Transport: &http.Transport{Proxy: nil}, CheckRedirect: func(_ *http.Request, _ []*http.Request) error { return http.ErrUseLastResponse }}
		defer client.CloseIdleConnections()
		res, err := client.Do(req)
		if err != nil {
			return 0, nil, err
		}
		defer res.Body.Close()
		body, err := io.ReadAll(io.LimitReader(res.Body, 4<<20))
		return res.StatusCode, body, err
	}
	defer func() {
		natural := f.pollMainExit()
		extra := map[string]any{"diagnostic_started_utc": started, "diagnostic_minimum_seconds": 100, "diagnostic_deadline_utc": until, "natural_exit_observed": natural, "harness_shutdown_not_yet_requested": true}
		if natural {
			extra["natural_exit_utc"] = f.mainExit.observed
			extra["natural_exit_error"] = fmt.Sprint(f.mainExit.err)
		}
		snapshot := &exec.Cmd{Process: f.main.Process}
		if natural {
			snapshot = f.main
		}
		f.processReceipt(phase+"-observation", snapshot, f.mainStarted, nil, extra)
	}()
	for {
		if f.pollMainExit() {
			f.t.Errorf("%s natural exit during diagnostic window: %v", phase, f.mainExit.err)
			diagnosticOK = false
			break
		}
		for _, path := range []string{"/readyz", "/api/v1/providers"} {
			status, rows, err := check(path)
			f.requestSeq++
			raw, _ := json.MarshalIndent(map[string]any{"method": "GET", "path": path, "status": status, "body": string(rows), "error": fmt.Sprint(err), "observed_utc": time.Now().UTC(), "diagnostic_only": true}, "", "  ")
			f.log(fmt.Sprintf("http-%03d-diagnostic.json", f.requestSeq), raw)
			if err != nil || status != 200 || (path == "/api/v1/providers" && before != nil && !bytes.Equal(before, rows)) {
				if diagnosticOK {
					f.t.Errorf("diagnostic readiness/readback FAILED: %s HTTP %d %v; leaving main alive for evidence", path, status, err)
				}
				diagnosticOK = false
			}
		}
		if !time.Now().UTC().Before(until) {
			break
		}
		select {
		case <-f.ctx.Done():
			f.t.Errorf("diagnostic externally canceled at %s: %v", time.Now().UTC().Format(time.RFC3339Nano), f.ctx.Err())
			return
		case <-time.After(2 * time.Second):
		}
	}
	if diagnosticOK {
		f.log(phase+"-observation-PASS.txt", []byte("alive, ready and stable readbacks for at least 100s after readiness; no harness termination in window\n"))
	} else {
		f.log(phase+"-observation-FAIL.txt", []byte("diagnostic failure preserved; main not terminated for readiness/readback failure before observation ended\n"))
	}
}
func (f *independentFixture) stop() (result error) {
	if f.main == nil {
		return nil
	}
	c := f.main
	if c.Process == nil {
		f.main = nil
		return nil
	}
	if f.mainDone == nil {
		f.mainDone = make(chan independentExit, 1)
		done := f.mainDone
		go func() { done <- independentExit{err: c.Wait(), observed: time.Now().UTC()} }()
	}
	requested := time.Now().UTC()
	var killedAt time.Time
	natural := f.pollMainExit()
	var termErr error
	if !natural {
		termErr = c.Process.Signal(syscall.SIGTERM)
	}
	defer func() {
		f.log(f.mainPhase+"-close.txt", []byte(fmt.Sprintf("pid=%d state=%v error=%v\n", c.Process.Pid, c.ProcessState, result)))
		f.processReceipt(f.mainPhase+"-shutdown", c, f.mainStarted, result, map[string]any{"shutdown_requested_utc": requested, "requested_signal": map[bool]string{true: "none (natural exit already observed)", false: "SIGTERM"}[natural], "natural_exit_before_shutdown": natural, "signal_error": fmt.Sprint(termErr), "forced_kill_utc": killedAt, "graceful_exit_proven": !natural && result == nil && termErr == nil && c.ProcessState != nil && c.ProcessState.ExitCode() == 0})
		f.main = nil
		f.mainDone = nil
		f.mainExit = nil
	}()
	if natural {
		return fmt.Errorf("main exited naturally before requested shutdown: %v", f.mainExit.err)
	}
	select {
	case exit := <-f.mainDone:
		if exit.err == nil && termErr != nil {
			return fmt.Errorf("graceful shutdown request not delivered: %w", termErr)
		}
		return exit.err
	case <-time.After(35 * time.Second):
		killedAt = time.Now().UTC()
		_ = c.Process.Kill()
		<-f.mainDone
		return fmt.Errorf("main exceeded clean shutdown deadline")
	}
}
func (f *independentFixture) helper(phase string, label ...string) {
	f.t.Helper()
	previous := f.env
	f.env = append(append([]string{}, f.env...), "GRANT_INDEPENDENT_HELPER=1", "GRANT_INDEPENDENT_HELPER_ROOT="+f.dir, "OWNED_PHASE="+phase, "OWNED_EXPECTED="+filepath.Join(f.logs, "ownership-etag.txt"))
	defer func() { f.env = previous }()
	logPhase := phase
	if len(label) != 0 {
		logPhase = label[0]
	}
	f.requireCommand("authority-"+logPhase, 60*time.Second, os.Args[0], "-test.run=^TestIndependentObjectDRHelper$", "-test.v", "-test.timeout=55s")
}
func (f *independentFixture) cleanup() {
	if err := f.stop(); err != nil {
		f.t.Errorf("cleanup main: %v", err)
	}
	if f.cid != "" || f.createAdmitted {
		// Cleanup remains live after the test/signal context expires.
		ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
		defer cancel()
		run := func(args ...string) ([]byte, error) {
			c := exec.CommandContext(ctx, "docker", args...)
			c.Env = f.env
			return c.CombinedOutput()
		}
		if f.cid == "" {
			var err error
			f.cid, err = independentDiscoverContainer(run, f.id)
			if err != nil {
				f.t.Errorf("unknown create reconciliation: %v", err)
				return
			}
			if f.cid == "" {
				f.log("create-reconciliation.txt", []byte("unknown create: successful exact name/label lookup found no container; original create remains failed\n"))
			}
		}
		if f.cid != "" {
			if out, e := run("inspect", "--format", "{{index .Config.Labels \"grant.independent.owner\"}}", f.cid); e != nil || strings.TrimSpace(string(out)) != f.id {
				f.t.Errorf("cleanup ownership check: %v %s", e, out)
				return
			}
			if out, e := run("inspect", "--format", "{{.Name}}", f.cid); e != nil || strings.TrimSpace(string(out)) != "/"+f.id {
				f.t.Errorf("cleanup container name mismatch: %v %s", e, out)
				return
			}
			f.log("container-id.txt", []byte(f.cid+"\n"))
			out, _ := run("logs", f.cid)
			f.log("emulator.log", out)
			if out, e := run("rm", "-f", "-v", f.cid); e != nil {
				f.t.Errorf("cleanup container: %v %s", e, out)
				return
			}
		}
		if out, e := run("ps", "-aq", "--no-trunc"); !independentContainerAbsent(out, e, f.cid) {
			f.t.Errorf("container absence unverified: %v %s", e, out)
			return
		}
	}
	if e := os.RemoveAll(f.dir); e != nil {
		f.t.Errorf("cleanup fixture: %v", e)
		return
	}
	f.log("cleanup.txt", []byte("owned main waited; successful Docker reconciliation/absence checks; owned runtime tree removed\n"))
}

func TestIndependentObjectDR(t *testing.T) {
	if os.Getenv("GRANT_INDEPENDENT_OBJECT_DR") != "1" {
		t.Skip("UNVERIFIED integration: run scripts/verify-independent-object-dr.sh")
	}
	if !strings.HasPrefix(os.Getenv("DOCKER_HOST"), "unix:///") {
		t.Fatal("only a local Unix Docker socket is allowed")
	}
	if !independentNativeSocket(os.Getenv("DOCKER_HOST")) {
		t.Fatal("UNVERIFIED: proxy/unknown daemon; independent native ownership requires owner review")
	}
	// Defense in depth for direct opt-in go test, before any provider pull/create.
	config, err := os.MkdirTemp("", "grant-native-config-")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = os.RemoveAll(config) })
	identityCtx, identityCancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer identityCancel()
	identity := func(args ...string) string {
		cmd := exec.CommandContext(identityCtx, "docker", args...)
		cmd.Env = []string{"PATH=" + os.Getenv("PATH"), "DOCKER_HOST=" + os.Getenv("DOCKER_HOST"), "DOCKER_CONFIG=" + config}
		out, err := cmd.CombinedOutput()
		if err != nil {
			t.Fatalf("UNVERIFIED native daemon identity: %v", err)
		}
		return strings.TrimSpace(string(out))
	}
	if !independentNativeServer(identity("version", "--format", "{{.Server.Platform.Name}}"), identity("info", "--format", "{{.OperatingSystem}}")) {
		t.Fatal("UNVERIFIED unknown daemon identity requires owner review")
	}
	binary := os.Getenv("GRANT_INDEPENDENT_BINARY")
	logs := os.Getenv("GRANT_INDEPENDENT_EVIDENCE")
	if !filepath.IsAbs(binary) || !filepath.IsAbs(logs) {
		t.Fatal("absolute owned binary/evidence paths required")
	}
	ctx, cancel := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer cancel()
	scenarios := append([]struct{ name, image string }{}, independentImages...)
	scenarios = append(scenarios, struct{ name, image string }{"gcs-scale", independentImages[1].image})
	for _, provider := range scenarios {
		t.Run(provider.name, func(t *testing.T) {
			providerName := strings.TrimSuffix(provider.name, "-scale")
			dir, err := os.MkdirTemp("", "grant-independent-"+provider.name+"-")
			if err != nil {
				t.Fatal(err)
			}
			f := &independentFixture{t: t, ctx: ctx, dir: dir, logs: filepath.Join(logs, provider.name), binary: binary, id: "grant-" + independentToken()}
			if err = os.WriteFile(filepath.Join(dir, "owner"), []byte(f.id), 0600); err != nil {
				t.Fatal(err)
			}
			if err = os.Mkdir(f.logs, 0700); err != nil {
				t.Fatal(err)
			}
			f.log("owner-id.txt", []byte(f.id))
			t.Cleanup(f.cleanup)
			// Never inherit provider credentials, proxies, kubeconfig, or consumer configuration.
			f.env = []string{"PATH=" + os.Getenv("PATH"), "DOCKER_CONFIG=" + filepath.Join(dir, "docker-config"), "TMPDIR=" + dir, "DOCKER_HOST=" + os.Getenv("DOCKER_HOST"), "GOMAXPROCS=2", "GOENV=off"}
			idp := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				t.Errorf("unexpected IdP request during internal-user fixture: %s", r.URL.Path)
				http.Error(w, "fixture does not authenticate external users", http.StatusUnauthorized)
			}))
			t.Cleanup(idp.Close)
			f.env = append(f.env, "GRANT_RAUTHY_BASE_URL="+idp.URL)
			if err = os.Mkdir(filepath.Join(dir, "docker-config"), 0700); err != nil {
				t.Fatal(err)
			}
			if _, e := f.docker("image", "inspect", provider.image); e != nil {
				f.requireCommand("pull", 60*time.Second, "docker", "pull", provider.image)
			}
			out, e := f.docker("image", "inspect", provider.image)
			f.log("image.json", out)
			if e != nil {
				t.Fatal(e)
			}
			port := map[string]string{"s3": "4566", "gcs": "4588", "azure": "4577"}[providerName]
			if err := independentAdmitCreate(f.docker, f.id); err != nil {
				t.Fatalf("create admission: %v", err)
			}
			f.log("create-admitted.txt", []byte(f.id))
			f.createAdmitted = true
			out, e = f.docker("create", "--name", f.id, "--pull=never", "--label", "grant.independent.owner="+f.id, "--memory=384m", "--cpus=1", "--pids-limit=128", "--log-opt", "max-size=2m", "--log-opt", "max-file=1", "-p", independentPort(t)+":"+port, provider.image)
			if e != nil {
				t.Fatalf("create: %v %s", e, out)
			}
			f.cid = strings.TrimSpace(string(out))
			f.log("container-id.txt", out)
			bindings, e := f.docker("inspect", "--format", "{{json .HostConfig.PortBindings}}", f.cid)
			f.log("requested-bindings.json", bindings)
			if e != nil {
				t.Fatal(e)
			}
			if e = independentLoopbackBindings(bindings); e != nil {
				t.Fatalf("UNVERIFIED local Docker cannot preserve loopback binding: %v", e)
			}
			f.requireCommand("emulator-start", 20*time.Second, "docker", "start", f.cid)
			inspect, inspectErr := f.docker("inspect", f.cid)
			f.log("container.json", inspect)
			if inspectErr != nil {
				t.Fatal(inspectErr)
			}
			out, e = f.docker("port", f.cid, port+"/tcp")
			if e != nil {
				t.Fatal(e)
			}
			addr := strings.TrimSpace(string(out))
			host, _, e := net.SplitHostPort(addr)
			if e != nil || host != "127.0.0.1" {
				t.Fatalf("non-loopback Docker binding %q", addr)
			}
			endpoint := "http://" + addr
			f.provision(providerName, endpoint)
			bind := independentPort(t)
			f.base = "http://" + bind
			cfg := map[string]any{"Local": true, "NodeID": f.id, "ClusterID": f.id, "Members": []any{}, "PeerAddr": "", "ObjStoreProvider": providerName, "ObjStoreBucket": f.id, "ObjStorePrefix": f.id}
			if provider.name == "s3" {
				cfg["ObjStoreEndpoint"] = addr
				cfg["ObjStoreInsecure"] = true
				cfg["ObjStoreRegion"] = "us-east-1"
				cfg["ObjStoreAccessKey"] = "fake"
				cfg["ObjStoreSecretKey"] = "fake"
			}
			if provider.name == "azure" {
				cfg["ObjStoreAzureStorageAccount"] = "devstoreaccount1"
				cfg["ObjStoreAzureConnectionString"] = independentAzureConnection(endpoint)
			}
			raw, _ := json.Marshal(cfg)
			f.env = append(f.env, "GRANT_RHIZA_CONFIG_JSON="+string(raw), "GRANT_DATA_DIR="+filepath.Join(dir, "data"), "GRANT_BACKUP_DIR="+filepath.Join(dir, "backup"), "GRANT_NODE_ID="+f.id, "GRANT_CLUSTER_ID="+f.id, "GRANT_CLUSTER_MEMBERS=[]", "GRANT_PEER_ADDR=", "GRANT_RUNTIME_MODE=embedded", "GRANT_LOCAL_NODE_ROLE=voter", "RHIZA_OBJSTORE_DURABILITY=before-ack", "GRANT_BIND_ADDR="+bind, "GRANT_BOOTSTRAP_USER_ID=owner", "GRANT_MASTER_KEY="+f.id+"-master", "GRANT_APP_AUTH_KEY="+f.id+"-app", "GRANT_RAUTHY_CLIENT_ID=fake", "GRANT_RAUTHY_CLIENT_SECRET=fake")
			if provider.name == "gcs-scale" {
				f.env = append(f.env, "GRANT_INDEPENDENT_GCS_SCALE=1")
			}
			f.requireCommand("init", 45*time.Second, binary, "restore-local-storage")
			f.start("main")
			if err = f.stop(); err != nil {
				t.Fatal(err)
			}
			f.helper("seed")
			f.start("seeded")
			marker := "dr-provider"
			if provider.name == "gcs-scale" {
				marker = f.id + "-provider-0"
			}
			write := f.request("POST", "/api/v1/admin/providers", map[string]any{"id": marker, "name": "Owned DR marker", "active": true}, 201)
			f.log("http-write.json", write)
			if provider.name == "gcs-scale" {
				for i := 0; i < 6; i++ {
					id := fmt.Sprintf("%s-provider-%d", f.id, i)
					if i != 0 {
						f.log(fmt.Sprintf("provider-%d-write.json", i), f.request("POST", "/api/v1/admin/providers", map[string]any{"id": id, "name": "Synthetic provider", "active": true}, 201))
					}
					pep := fmt.Sprintf("%s-pep-%d", f.id, i)
					f.log(fmt.Sprintf("pep-%d-write.json", i), f.request("PUT", "/ops/providers/"+id+"/enforcers/"+pep, UpsertProviderPepClient{Lifecycle: "active"}, 200))
				}
			}
			before := f.request("GET", "/api/v1/providers", nil, 200)
			f.log("http-before.json", before)
			if err = f.stop(); err != nil {
				t.Fatal(err)
			}
			f.start("restart")
			restart := f.request("GET", "/api/v1/providers", nil, 200)
			if !bytes.Equal(before, restart) {
				t.Fatal("restart changed provider data")
			}
			if err = f.stop(); err != nil {
				t.Fatal(err)
			}
			f.helper("verify")
			cycles := 1
			if provider.name == "gcs-scale" {
				cycles = 2
			}
			providerCount, pepCount := 1, 0
			if provider.name == "gcs-scale" {
				providerCount, pepCount = 6, 6
			}
			f.log("scenario.txt", []byte(fmt.Sprintf("scenario=%s cold-loss-cycles=%d synthetic-providers=%d active-peps=%d\n", provider.name, cycles, providerCount, pepCount)))
			for cycle := 1; cycle <= cycles; cycle++ {
				if err = os.RemoveAll(filepath.Join(dir, "data")); err != nil {
					t.Fatal(err)
				}
				if _, err = os.Stat(filepath.Join(dir, "data")); !os.IsNotExist(err) {
					t.Fatal("entire DataDir was not removed")
				}
				f.log(fmt.Sprintf("datadir-loss-%d.txt", cycle), []byte("entire DataDir removed and absence verified after clean shutdown\n"))
				f.requireCommand(fmt.Sprintf("restore-%d", cycle), 45*time.Second, binary, "restore-local-storage")
				f.start(fmt.Sprintf("restored-%d", cycle))
				f.observeRecovery(fmt.Sprintf("restored-%d", cycle), before)
				after := f.request("GET", "/api/v1/providers", nil, 200)
				f.log(fmt.Sprintf("http-restored-%d.json", cycle), after)
				if !bytes.Equal(before, after) || !bytes.Contains(after, []byte(marker)) {
					t.Fatal("restored provider rows mismatch")
				}
				if err = f.stop(); err != nil {
					t.Fatal(err)
				}
				f.helper("verify-restored", fmt.Sprintf("verify-restored-%d", cycle))
			}
			if t.Failed() {
				return
			}
			f.log("PASS", []byte("PASS process init/main/http/restart/full-DataDir-loss/restore; role/action/binding/owner-ETag/allow-deny continuity\n"))
			t.Log("PASS process DR and authority; emulator only; no fencing/cloud/Kubernetes claim")
		})
	}
}

func independentNativeSocket(endpoint string) bool {
	lower := strings.ToLower(endpoint)
	for _, forbidden := range []string{"dory", "bridge", "proxy", "kube", "k8s", "rancher", "colima", "podman"} {
		if strings.Contains(lower, forbidden) {
			return false
		}
	}
	return endpoint == "unix:///var/run/docker.sock" || (strings.HasPrefix(endpoint, "unix:///") && strings.HasSuffix(endpoint, "/.docker/run/docker.sock"))
}
func independentNativeServer(platform, operatingSystem string) bool {
	known := platform == "Docker Engine - Community" || platform == "Docker Engine - Enterprise" || platform == "Docker Engine" || platform == "Moby Engine" || platform == "Docker Desktop" || strings.HasPrefix(platform, "Docker Desktop ")
	if !known {
		return false
	}
	for _, prefix := range []string{"Ubuntu", "Debian", "Docker Desktop", "Alpine Linux", "Fedora Linux", "CentOS", "Rocky Linux", "Amazon Linux"} {
		if strings.HasPrefix(operatingSystem, prefix) {
			return true
		}
	}
	return false
}
func TestIndependentObjectDRNativeBoundary(t *testing.T) {
	for _, tc := range []struct {
		socket string
		valid  bool
	}{{"unix:///var/run/docker.sock", true}, {"unix:///Users/test/.docker/run/docker.sock", true}, {"unix:///Users/test/.dory/dory.sock", false}, {"tcp://127.0.0.1:2375", false}, {"unix:///tmp/unknown.sock", false}} {
		if independentNativeSocket(tc.socket) != tc.valid {
			t.Errorf("socket boundary %q", tc.socket)
		}
	}
	if !independentNativeServer("Docker Engine - Community", "Ubuntu 24.04") || !independentNativeServer("Docker Desktop 4.0", "Docker Desktop") || independentNativeServer("Dory bridge", "Ubuntu 24.04") || independentNativeServer("Docker Engine - Community", "unknown") {
		t.Fatal("native identity boundary")
	}
}

func independentContainsCID(out []byte, cid string) bool {
	for _, line := range strings.Fields(string(out)) {
		if line == cid {
			return true
		}
	}
	return false
}

func independentContainerAbsent(out []byte, err error, cid string) bool {
	return err == nil && !independentContainsCID(out, cid)
}

func TestIndependentObjectDRContainerAbsence(t *testing.T) {
	for _, tc := range []struct {
		out    string
		err    error
		absent bool
	}{
		{"", nil, true},
		{"other\n", nil, true},
		{"owned\n", nil, false},
		{"", fmt.Errorf("daemon unavailable"), false},
	} {
		if independentContainerAbsent([]byte(tc.out), tc.err, "owned") != tc.absent {
			t.Errorf("absence oracle out=%q err=%v", tc.out, tc.err)
		}
	}
}

const independentAzureKey = "Eby8vdM02xNOcqFlqUwJPLlmEtlCDXJ1OUzFT50uSRZ6IFsuFq2UVErCz4I6tq/K1SZFPTOtr/KBHBeksoGMGw=="

func independentAzureConnection(endpoint string) string {
	return "DefaultEndpointsProtocol=http;AccountName=devstoreaccount1;AccountKey=" + independentAzureKey + ";BlobEndpoint=" + endpoint + "/devstoreaccount1;"
}
func (f *independentFixture) provision(provider, endpoint string) {
	f.t.Helper()
	ctx, cancel := context.WithTimeout(f.ctx, 30*time.Second)
	defer cancel()
	// Readiness is an actual SDK bucket/container create, retried only on this owned endpoint.
	var create func() error
	switch provider {
	case "s3":
		c, e := minio.New(strings.TrimPrefix(endpoint, "http://"), &minio.Options{Creds: credentials.NewStaticV4("fake", "fake", ""), Secure: false, Region: "us-east-1"})
		if e != nil {
			f.t.Fatal(e)
		}
		create = func() error { return c.MakeBucket(ctx, f.id, minio.MakeBucketOptions{Region: "us-east-1"}) }
	case "azure":
		c, e := azblob.NewClientFromConnectionString(independentAzureConnection(endpoint), nil)
		if e != nil {
			f.t.Fatal(e)
		}
		create = func() error { _, e := c.CreateContainer(ctx, f.id, nil); return e }
	case "gcs":
		key, e := rsa.GenerateKey(rand.Reader, 2048)
		if e != nil {
			f.t.Fatal(e)
		}
		adc := map[string]string{"type": "service_account", "project_id": "grant-owned", "private_key_id": "fake", "client_email": "owned@grant-owned.iam.gserviceaccount.com", "token_uri": endpoint + "/token", "private_key": string(pem.EncodeToMemory(&pem.Block{Type: "RSA PRIVATE KEY", Bytes: x509.MarshalPKCS1PrivateKey(key)}))}
		raw, _ := json.Marshal(adc)
		p := filepath.Join(f.dir, "fake-adc.json")
		if e = os.WriteFile(p, raw, 0600); e != nil {
			f.t.Fatal(e)
		}
		f.env = append(f.env, "STORAGE_EMULATOR_HOST="+endpoint, "GOOGLE_APPLICATION_CREDENTIALS="+p)
		// SDK construction must see the emulator variable; each provider runs serially.
		f.t.Setenv("STORAGE_EMULATOR_HOST", endpoint)
		c, e := storage.NewClient(ctx, option.WithoutAuthentication())
		if e != nil {
			f.t.Fatal(e)
		}
		defer c.Close()
		create = func() error { return c.Bucket(f.id).Create(ctx, "grant-owned", nil) }
	}
	for {
		if e := create(); e == nil {
			return
		} else {
			f.log("provision-last-error.txt", []byte(e.Error()))
		}
		select {
		case <-ctx.Done():
			f.t.Fatalf("UNVERIFIED %s provisioning: %v", provider, ctx.Err())
		case <-time.After(250 * time.Millisecond):
		}
	}
}

func TestIndependentDockerStdout(t *testing.T) {
	dir := t.TempDir()
	docker := filepath.Join(dir, "docker")
	if err := os.WriteFile(docker, []byte("#!/bin/sh\nprintf '%s\\n' \"$1\"\nprintf '%s\\n' 'WARNING: test diagnostic' >&2\n"), 0700); err != nil {
		t.Fatal(err)
	}
	t.Setenv("PATH", dir)
	f := &independentFixture{t: t, ctx: context.Background(), logs: dir, env: []string{"PATH=" + dir}}
	cid := strings.Repeat("a", 64)
	for _, value := range []string{cid, "127.0.0.1:12345"} {
		out, err := f.docker(value)
		if err != nil || string(out) != value+"\n" {
			t.Fatalf("stdout contaminated: %q, %v", out, err)
		}
	}
	stderr, err := os.ReadFile(filepath.Join(dir, "docker.stderr.log"))
	if err != nil || string(stderr) != strings.Repeat("WARNING: test diagnostic\n", 2) {
		t.Fatalf("stderr not preserved: %q, %v", stderr, err)
	}
}

func TestIndependentObjectDRPins(t *testing.T) {
	for _, p := range independentImages {
		parts := strings.Split(p.image, "@sha256:")
		if len(parts) != 2 || len(parts[1]) != sha256.Size*2 {
			t.Fatalf("unpinned provider %s", p.name)
		}
		if _, e := hex.DecodeString(parts[1]); e != nil {
			t.Fatal(e)
		}
	}
}

func independentLoopbackBindings(raw []byte) error {
	var ports map[string][]struct {
		HostIP string `json:"HostIp"`
	}
	if err := json.Unmarshal(raw, &ports); err != nil {
		return err
	}
	if len(ports) != 1 {
		return fmt.Errorf("expected one published port")
	}
	for _, bindings := range ports {
		if len(bindings) != 1 || bindings[0].HostIP != "127.0.0.1" {
			return fmt.Errorf("binding must remain exactly 127.0.0.1")
		}
	}
	return nil
}

func TestIndependentObjectDRBindingGuard(t *testing.T) {
	for _, tc := range []struct {
		raw string
		ok  bool
	}{
		{`{"4566/tcp":[{"HostIp":"127.0.0.1","HostPort":"1234"}]}`, true},
		{`{"4566/tcp":[{"HostIp":"","HostPort":"1234"}]}`, false},
		{`{"4566/tcp":[{"HostIp":"0.0.0.0","HostPort":"1234"}]}`, false},
		{`{"4566/tcp":[{"HostIp":"192.0.2.1","HostPort":"1234"}]}`, false},
		{`{}`, false},
	} {
		if got := independentLoopbackBindings([]byte(tc.raw)) == nil; got != tc.ok {
			t.Errorf("binding guard %s accepted=%v", tc.raw, got)
		}
	}
}

// Admission is recorded before POST; a collision never arms cleanup ownership.
func independentAdmitCreate(run func(...string) ([]byte, error), owner string) error {
	cid, err := independentDiscoverContainer(run, owner)
	if err != nil {
		return err
	}
	if cid != "" {
		return fmt.Errorf("pre-existing exact owner/name collision")
	}
	return nil
}
func independentDiscoverContainer(run func(...string) ([]byte, error), owner string) (string, error) {
	label, err := run("ps", "-aq", "--no-trunc", "--filter", "label=grant.independent.owner="+owner)
	if err != nil {
		return "", err
	}
	name, err := run("ps", "-aq", "--no-trunc", "--filter", "name=^/"+owner+"$")
	if err != nil {
		return "", err
	}
	labels, names := strings.Fields(string(label)), strings.Fields(string(name))
	if len(labels) == 0 && len(names) == 0 {
		return "", nil
	}
	if len(labels) != 1 || len(names) != 1 || labels[0] != names[0] {
		return "", fmt.Errorf("ambiguous or foreign owner/name match")
	}
	if len(labels[0]) != 64 {
		return "", fmt.Errorf("invalid container ID")
	}
	if _, err := hex.DecodeString(labels[0]); err != nil {
		return "", err
	}
	return labels[0], nil
}
func TestIndependentObjectDRCreateReconciliation(t *testing.T) {
	cid := strings.Repeat("a", 64)
	for _, tc := range []struct {
		name, label, named string
		admit, discover    bool
	}{
		{"absent", "", "", true, true},
		{"lost-response-committed", cid, cid, false, true},
		{"pre-existing-name-only", "", cid, false, false},
		{"conflicting-label", cid, "", false, false},
		{"ambiguous", cid + "\n" + strings.Repeat("b", 64), cid, false, false},
	} {
		t.Run(tc.name, func(t *testing.T) {
			mock := func(args ...string) ([]byte, error) {
				if args[0] != "ps" {
					t.Fatalf("lookup/admission must not mutate Docker: %v", args)
				}
				if strings.HasPrefix(args[len(args)-1], "label=") {
					return []byte(tc.label), nil
				}
				return []byte(tc.named), nil
			}
			if got := independentAdmitCreate(mock, "grant-owned") == nil; got != tc.admit {
				t.Fatalf("admission=%v", got)
			}
			got, err := independentDiscoverContainer(mock, "grant-owned")
			if (err == nil) != tc.discover {
				t.Fatalf("discovery %q %v", got, err)
			}
			if tc.name == "lost-response-committed" && got != cid {
				t.Fatal("lost CID not reconciled")
			}
		})
	}
}
