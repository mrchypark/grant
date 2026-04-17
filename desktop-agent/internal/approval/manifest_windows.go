//go:build windows

package approval

// app_windows_amd64.syso embeds app.manifest into the Windows binary so the
// loader activates ComCtl32 v6 (modern visual styles + EM_SETCUEBANNER support)
// and per-monitor DPI v2. Regenerate after editing app.manifest:
//
//   go install github.com/akavel/rsrc@latest
//   cd internal/approval && rsrc -manifest app.manifest -arch amd64 -o app_windows_amd64.syso
//
//go:generate rsrc -manifest app.manifest -arch amd64 -o app_windows_amd64.syso
