//go:build darwin

package app

import (
	"context"
	"fmt"
	"net/http"
	"os/exec"
	"sync"
	"time"

	"github.com/cypark/account/desktop-agent/internal/localization"
	"github.com/getlantern/systray"
)

func run(addr string, handler http.Handler, _ RunOptions) error {
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
		copy := localization.ForLanguage(localization.SystemLanguage())
		templateIconBytes, regularIconBytes := trayIconBytes()
		systray.SetTemplateIcon(templateIconBytes, regularIconBytes)
		systray.SetTooltip(copy.AppTooltip)

		statusItem := systray.AddMenuItem(fmt.Sprintf(copy.StatusListening, addr), copy.AppTooltip)
		statusItem.Disable()
		openItem := systray.AddMenuItem(copy.OpenExamplePage, copy.OpenExampleTooltip)
		systray.AddSeparator()
		quitItem := systray.AddMenuItem(copy.QuitApp, copy.QuitAppTooltip)

		go func() {
			for {
				select {
				case <-openItem.ClickedCh:
					_ = exec.Command("open", examplePageURL()).Start()
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
