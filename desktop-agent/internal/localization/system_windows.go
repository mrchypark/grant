//go:build windows

package localization

import (
	"syscall"
	"unsafe"
)

const windowsLocaleNameMaxLength = 85

var kernel32 = syscall.NewLazyDLL("kernel32.dll")
var procGetUserDefaultLocaleName = kernel32.NewProc("GetUserDefaultLocaleName")

func SystemLanguage() Language {
	var buf [windowsLocaleNameMaxLength]uint16
	ret, _, _ := procGetUserDefaultLocaleName.Call(
		uintptr(unsafe.Pointer(&buf[0])),
		uintptr(len(buf)),
	)
	if ret == 0 {
		return Korean
	}
	return languageFromTag(syscall.UTF16ToString(buf[:]))
}
