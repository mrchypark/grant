//go:build darwin

package app

import (
	"context"
	"fmt"
	"net/http"
	"os"
	"os/exec"
	"sync"
	"time"

	"github.com/getlantern/systray"
)

func run(addr string, handler http.Handler, _ RunOptions) error {
	if !shouldUseTray("darwin", os.Getenv("DESKTOP_AGENT_DISABLE_TRAY")) {
		return newHTTPServer(addr, handler).ListenAndServe()
	}

	server := newHTTPServer(addr, handler)

	errCh := make(chan error, 1)
	quitCh := make(chan struct{})
	doneCh := make(chan struct{})
	shutdownErrCh := make(chan error, 1)
	var once sync.Once

	go func() {
		err := server.ListenAndServe()
		if err == nil || err == http.ErrServerClosed {
			errCh <- nil
			return
		}
		errCh <- err
	}()

	go func() {
		if err := <-errCh; err != nil {
			once.Do(func() {
				shutdownErrCh <- err
				close(doneCh)
				systray.Quit()
			})
		}
	}()

	onReady := func() {
		systray.SetTitle("certd")
		systray.SetTooltip("certd desktop agent")

		statusItem := systray.AddMenuItem(fmt.Sprintf("Agent listening on %s", addr), "agent status")
		statusItem.Disable()
		openItem := systray.AddMenuItem("Open Example Page", "open the local browser example")
		systray.AddSeparator()
		quitItem := systray.AddMenuItem("Quit certd Agent", "quit agent")

		go func() {
			for {
				select {
				case <-openItem.ClickedCh:
					_ = exec.Command("open", "http://127.0.0.1:4173/").Start()
				case <-quitItem.ClickedCh:
					once.Do(func() {
						close(quitCh)
						close(doneCh)
						systray.Quit()
					})
					return
				}
			}
		}()
	}

	onExit := func() {
		ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		_ = server.Shutdown(ctx)
	}

	systray.Run(onReady, onExit)
	select {
	case <-quitCh:
		return nil
	case <-doneCh:
		select {
		case err := <-shutdownErrCh:
			return err
		default:
			return nil
		}
	}
}
