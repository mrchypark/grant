package main

import (
	"fmt"
	"os"
	"os/signal"
	"syscall"
	"time"
)

func main() {
	if len(os.Args) == 2 && os.Args[1] == "--contract-check" {
		if err := verifyExecutableContract("/app/rauthy", []string{"serve", "--help"}, 10*time.Second); err != nil {
			fmt.Fprintf(os.Stderr, "rauthy-launcher contract: %v\n", err)
			os.Exit(1)
		}
		fmt.Println("rauthy-launcher contract ok")
		return
	}

	cfg, err := configFromEnv(os.Args[1:])
	if err != nil {
		fmt.Fprintf(os.Stderr, "rauthy-launcher: %v\n", err)
		os.Exit(2)
	}

	signals := make(chan os.Signal, 4)
	signal.Notify(signals, syscall.SIGTERM, syscall.SIGINT, syscall.SIGHUP, syscall.SIGQUIT)
	defer signal.Stop(signals)

	if err := run(cfg, signals); err != nil {
		fmt.Fprintf(os.Stderr, "rauthy-launcher: %v\n", err)
		if exited, ok := err.(interface{ ExitCode() int }); ok {
			os.Exit(exited.ExitCode())
		}
		os.Exit(1)
	}
}
