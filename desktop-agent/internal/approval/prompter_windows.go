//go:build windows

package approval

import (
	"bytes"
	"encoding/binary"
	"fmt"
	"strings"
	"sync"
	"syscall"
	"unicode/utf16"
	"unsafe"

	"golang.org/x/sys/windows"
)

// Win32 dialog/control style bits, messages, standard class atoms.
const (
	dsModalFrame uint32 = 0x00000080
	dsSetFont    uint32 = 0x00000040
	dsCenter     uint32 = 0x00000800

	wsPopup   uint32 = 0x80000000
	wsCaption uint32 = 0x00C00000
	wsSysMenu uint32 = 0x00080000
	wsVisible uint32 = 0x10000000
	wsChild   uint32 = 0x40000000
	wsTabStop uint32 = 0x00010000
	wsBorder  uint32 = 0x00800000
	wsVScroll uint32 = 0x00200000

	esPassword    uint32 = 0x0020
	esAutoHScroll uint32 = 0x0080

	ssLeft        uint32 = 0x00000000
	ssIcon        uint32 = 0x00000003
	ssEtchedHorz  uint32 = 0x00000010
	ssCenterImage uint32 = 0x00000200

	bsDefPushButton uint32 = 0x00000001
	bsPushButton    uint32 = 0x00000000

	cbsDropDownList uint32 = 0x00000003
	cbsHasStrings   uint32 = 0x00000200

	wmInitDialog uint32 = 0x0110
	wmCommand    uint32 = 0x0111
	wmDestroy    uint32 = 0x0002

	wmSetFont      uintptr = 0x0030
	stmSetIcon     uintptr = 0x0170
	emSetCueBanner uintptr = 0x1501

	cbAddString uintptr = 0x0143
	cbSetCurSel uintptr = 0x014E
	cbGetCurSel uintptr = 0x0147

	fwBold uint32 = 700

	idiWarning uintptr = 32515

	idOK     = 1
	idCancel = 2

	idcHeading  uint16 = 1000
	idcIcon     uint16 = 1001
	idcInfo     uint16 = 1002
	idcCombo    uint16 = 1003
	idcPassword uint16 = 1004

	classButton   uint16 = 0x0080
	classEdit     uint16 = 0x0081
	classStatic   uint16 = 0x0082
	classComboBox uint16 = 0x0085
)

var (
	user32                       = windows.NewLazySystemDLL("user32.dll")
	gdi32                        = windows.NewLazySystemDLL("gdi32.dll")
	kernel32                     = windows.NewLazySystemDLL("kernel32.dll")
	procDialogBoxIndirectParamW  = user32.NewProc("DialogBoxIndirectParamW")
	procEndDialog                = user32.NewProc("EndDialog")
	procSendDlgItemMessageW      = user32.NewProc("SendDlgItemMessageW")
	procSendMessageW             = user32.NewProc("SendMessageW")
	procGetDlgItemTextW          = user32.NewProc("GetDlgItemTextW")
	procGetWindowTextLengthW     = user32.NewProc("GetWindowTextLengthW")
	procGetDlgItem               = user32.NewProc("GetDlgItem")
	procSetFocus                 = user32.NewProc("SetFocus")
	procSetForegroundWindow      = user32.NewProc("SetForegroundWindow")
	procBringWindowToTop         = user32.NewProc("BringWindowToTop")
	procGetForegroundWindow      = user32.NewProc("GetForegroundWindow")
	procGetWindowThreadProcessId = user32.NewProc("GetWindowThreadProcessId")
	procAttachThreadInput        = user32.NewProc("AttachThreadInput")
	procAllowSetForegroundWindow = user32.NewProc("AllowSetForegroundWindow")
	procPostMessageW             = user32.NewProc("PostMessageW")
	procLoadIconW                = user32.NewProc("LoadIconW")
	procCreateFontW              = gdi32.NewProc("CreateFontW")
	procDeleteObject             = gdi32.NewProc("DeleteObject")
	procGetCurrentThreadId       = kernel32.NewProc("GetCurrentThreadId")
)

// 동시에 한 개 모달만 띄우도록 직렬화하는 state. darwin의 NSAlert main-queue
// dispatch와 동일한 효과.
var (
	win32ModalMu    sync.Mutex
	currentInput    approvalModalInput
	currentResult   approvalModalOutput
	headingFont     uintptr
	cancelMu        sync.Mutex
	currentHWND     uintptr
	shouldCancelW32 bool
)

var dialogProcCallback = syscall.NewCallback(dialogProc)

var showDialogBoxIndirectParam = func(template []byte) {
	procDialogBoxIndirectParamW.Call(
		0,
		uintptr(unsafe.Pointer(&template[0])),
		0,
		dialogProcCallback,
		0,
	)
}

var getDialogItemHandle = func(hwnd, id uintptr) uintptr {
	ret, _, _ := procGetDlgItem.Call(hwnd, id)
	return ret
}

var getWindowTextLength = func(hwnd uintptr) int {
	ret, _, _ := procGetWindowTextLengthW.Call(hwnd)
	return int(ret)
}

var getDialogItemText = func(hwnd, id uintptr, buf []uint16) int {
	if len(buf) == 0 {
		return 0
	}
	ret, _, _ := procGetDlgItemTextW.Call(hwnd, id, uintptr(unsafe.Pointer(&buf[0])), uintptr(len(buf)))
	return int(ret)
}

func init() {
	resetApprovalModalCancellation = func() {
		cancelMu.Lock()
		shouldCancelW32 = false
		cancelMu.Unlock()
	}
	requestApprovalModalCancellation = func() {
		cancelMu.Lock()
		shouldCancelW32 = true
		hwnd := currentHWND
		cancelMu.Unlock()
		if hwnd != 0 {
			procPostMessageW.Call(hwnd, uintptr(wmCommand), uintptr(idCancel), 0)
		}
	}
	activateApprovalModalApplication = func() {
		// ASFW_ANY(-1)로 foreground 쟁취 권한을 사전 승인받아 둔다. 실제 raise는
		// WM_INITDIALOG 안의 AttachThreadInput 경로에서 수행.
		procAllowSetForegroundWindow.Call(^uintptr(0))
	}
	runApprovalModal = showWin32Modal
}

func showWin32Modal(input approvalModalInput) approvalModalOutput {
	win32ModalMu.Lock()
	defer win32ModalMu.Unlock()

	template, err := buildDialogTemplate(input)
	if err != nil {
		return approvalModalOutput{Err: err}
	}

	currentInput = input
	currentResult = approvalModalOutput{}

	activateApprovalModalApplication()
	showDialogBoxIndirectParam(template)

	cancelMu.Lock()
	currentHWND = 0
	cancelMu.Unlock()

	result := currentResult
	currentResult = approvalModalOutput{}
	return result
}

func dialogProc(hwnd, msg, wparam, _ uintptr) uintptr {
	switch uint32(msg) {
	case wmInitDialog:
		cancelMu.Lock()
		currentHWND = hwnd
		abort := shouldCancelW32
		cancelMu.Unlock()
		if abort {
			procEndDialog.Call(hwnd, uintptr(idCancel))
			return 1
		}

		applyHeadingFont(hwnd)
		loadWarningIcon(hwnd)
		setPasswordCueBanner(hwnd, currentInput.Copy.PasswordPlaceholder)
		populateCombo(hwnd)
		raiseForeground(hwnd)

		pwHwnd, _, _ := procGetDlgItem.Call(hwnd, uintptr(idcPassword))
		procSetFocus.Call(pwHwnd)
		return 0
	case wmCommand:
		switch int(wparam & 0xFFFF) {
		case idOK:
			selRet, _, _ := procSendDlgItemMessageW.Call(hwnd, uintptr(idcCombo), cbGetCurSel, 0, 0)
			sel := int32(selRet)
			password := readDlgItemText(hwnd, uintptr(idcPassword))
			currentResult = approvalModalOutput{
				SelectedIndex: int(sel),
				Password:      password,
				Approved:      true,
			}
			procEndDialog.Call(hwnd, uintptr(idOK))
			return 1
		case idCancel:
			currentResult = approvalModalOutput{}
			procEndDialog.Call(hwnd, uintptr(idCancel))
			return 1
		}
	case wmDestroy:
		if headingFont != 0 {
			procDeleteObject.Call(headingFont)
			headingFont = 0
		}
	}
	return 0
}

func applyHeadingFont(hwnd uintptr) {
	faceName, _ := syscall.UTF16PtrFromString("Segoe UI")
	// 음수 height = character height in logical units. -16 ≈ 12pt on 96 DPI.
	var h int32 = -16
	headingFont, _, _ = procCreateFontW.Call(
		uintptr(uint32(h)), 0, 0, 0,
		uintptr(fwBold), 0, 0, 0, 1, 0, 0, 0, 0,
		uintptr(unsafe.Pointer(faceName)),
	)
	if headingFont == 0 {
		return
	}
	headingHwnd, _, _ := procGetDlgItem.Call(hwnd, uintptr(idcHeading))
	procSendMessageW.Call(headingHwnd, wmSetFont, headingFont, 1)
}

func loadWarningIcon(hwnd uintptr) {
	hicon, _, _ := procLoadIconW.Call(0, idiWarning)
	if hicon == 0 {
		return
	}
	iconHwnd, _, _ := procGetDlgItem.Call(hwnd, uintptr(idcIcon))
	procSendMessageW.Call(iconHwnd, stmSetIcon, hicon, 0)
}

func setPasswordCueBanner(hwnd uintptr, placeholder string) {
	if strings.TrimSpace(placeholder) == "" {
		return
	}
	cue, err := syscall.UTF16PtrFromString(placeholder)
	if err != nil {
		return
	}
	pwHwnd, _, _ := procGetDlgItem.Call(hwnd, uintptr(idcPassword))
	// EM_SETCUEBANNER wParam=TRUE: focus가 있어도 cue 유지. ComCtl32 v6 필요.
	procSendMessageW.Call(pwHwnd, emSetCueBanner, 1, uintptr(unsafe.Pointer(cue)))
}

func populateCombo(hwnd uintptr) {
	for _, label := range currentInput.Labels {
		ptr, err := syscall.UTF16PtrFromString(label)
		if err != nil {
			continue
		}
		procSendDlgItemMessageW.Call(hwnd, uintptr(idcCombo), cbAddString, 0, uintptr(unsafe.Pointer(ptr)))
	}
	idx := currentInput.DefaultIndex
	if idx < 0 || idx >= len(currentInput.Labels) {
		idx = 0
	}
	procSendDlgItemMessageW.Call(hwnd, uintptr(idcCombo), cbSetCurSel, uintptr(idx), 0)
}

// raiseForeground defeats foreground-lock by temporarily attaching this thread's
// input queue to the thread that currently owns the foreground window. Tray
// processes otherwise only flash the taskbar when calling SetForegroundWindow.
func raiseForeground(hwnd uintptr) {
	fgHwnd, _, _ := procGetForegroundWindow.Call()
	if fgHwnd == 0 {
		procSetForegroundWindow.Call(hwnd)
		return
	}
	fgThread, _, _ := procGetWindowThreadProcessId.Call(fgHwnd, 0)
	curThread, _, _ := procGetCurrentThreadId.Call()
	if fgThread == 0 || fgThread == curThread {
		procSetForegroundWindow.Call(hwnd)
		return
	}
	procAttachThreadInput.Call(curThread, fgThread, 1)
	procBringWindowToTop.Call(hwnd)
	procSetForegroundWindow.Call(hwnd)
	procAttachThreadInput.Call(curThread, fgThread, 0)
}

func readDlgItemText(hwnd, id uintptr) string {
	item := getDialogItemHandle(hwnd, id)
	if item == 0 {
		return ""
	}
	bufLen := getWindowTextLength(item) + 1
	if bufLen < 2 {
		bufLen = 2
	}
	buf := make([]uint16, bufLen)
	n := getDialogItemText(hwnd, id, buf)
	return syscall.UTF16ToString(buf[:n])
}

type ctrlSpec struct {
	style  uint32
	x, y   int16
	cx, cy int16
	id     uint16
	class  uint16
	text   string
}

// buildDialogTemplate composes an in-memory DLGTEMPLATE. 좌표는 dialog units
// (DLU). Segoe UI 9pt에서 1 DLU ≈ 1.75px(가로)/2px(세로).
func buildDialogTemplate(input approvalModalInput) ([]byte, error) {
	const dlgW, dlgH = 300, 220

	info := fmt.Sprintf(
		input.Copy.ApprovalInfoFormat,
		input.Origin,
		input.OrganizationID,
		input.BusinessNo,
		input.Action,
	)
	// Win32 static는 \n을 개행으로 그리지만 가끔 \r\n이 안전하다.
	info = strings.ReplaceAll(info, "\r\n", "\n")
	info = strings.ReplaceAll(info, "\n", "\r\n")

	controls := []ctrlSpec{
		iconSlot(16, 12, 20, 20, idcIcon),
		staticLabel(44, 14, 240, 18, input.Copy.ApprovalTitle, idcHeading),
		staticLabel(16, 38, 270, 72, info, idcInfo),
		etchedSeparator(16, 114, 270, 1),
		staticLabel(16, 122, 100, 10, input.Copy.CertificateLabel, 0),
		comboBox(16, 134, 270, 120, idcCombo),
		staticLabel(16, 154, 100, 10, input.Copy.PasswordLabel, 0),
		passwordEdit(16, 166, 270, 14, idcPassword),
		button(76, 192, 70, 18, idOK, input.Copy.Approve, true),
		button(156, 192, 70, 18, idCancel, input.Copy.Cancel, false),
	}

	var buf bytes.Buffer
	dlgStyle := dsModalFrame | dsSetFont | dsCenter | wsPopup | wsCaption | wsSysMenu
	_ = binary.Write(&buf, binary.LittleEndian, dlgStyle)
	_ = binary.Write(&buf, binary.LittleEndian, uint32(0))
	_ = binary.Write(&buf, binary.LittleEndian, uint16(len(controls)))
	_ = binary.Write(&buf, binary.LittleEndian, int16(0))
	_ = binary.Write(&buf, binary.LittleEndian, int16(0))
	_ = binary.Write(&buf, binary.LittleEndian, int16(dlgW))
	_ = binary.Write(&buf, binary.LittleEndian, int16(dlgH))
	_ = binary.Write(&buf, binary.LittleEndian, uint16(0))
	_ = binary.Write(&buf, binary.LittleEndian, uint16(0))
	writeUTF16(&buf, input.Copy.ApprovalTitle)
	_ = binary.Write(&buf, binary.LittleEndian, uint16(9))
	writeUTF16(&buf, "Segoe UI")

	for _, c := range controls {
		alignDword(&buf)
		_ = binary.Write(&buf, binary.LittleEndian, c.style)
		_ = binary.Write(&buf, binary.LittleEndian, uint32(0))
		_ = binary.Write(&buf, binary.LittleEndian, c.x)
		_ = binary.Write(&buf, binary.LittleEndian, c.y)
		_ = binary.Write(&buf, binary.LittleEndian, c.cx)
		_ = binary.Write(&buf, binary.LittleEndian, c.cy)
		_ = binary.Write(&buf, binary.LittleEndian, c.id)
		_ = binary.Write(&buf, binary.LittleEndian, uint16(0xFFFF))
		_ = binary.Write(&buf, binary.LittleEndian, c.class)
		writeUTF16(&buf, c.text)
		_ = binary.Write(&buf, binary.LittleEndian, uint16(0))
	}

	return buf.Bytes(), nil
}

func staticLabel(x, y, cx, cy int16, text string, id uint16) ctrlSpec {
	return ctrlSpec{
		style: wsChild | wsVisible | ssLeft,
		x:     x, y: y, cx: cx, cy: cy,
		id:    id,
		class: classStatic,
		text:  text,
	}
}

func iconSlot(x, y, cx, cy int16, id uint16) ctrlSpec {
	return ctrlSpec{
		style: wsChild | wsVisible | ssIcon | ssCenterImage,
		x:     x, y: y, cx: cx, cy: cy,
		id:    id,
		class: classStatic,
	}
}

func etchedSeparator(x, y, cx, cy int16) ctrlSpec {
	return ctrlSpec{
		style: wsChild | wsVisible | ssEtchedHorz,
		x:     x, y: y, cx: cx, cy: cy,
		class: classStatic,
	}
}

func comboBox(x, y, cx, cy int16, id uint16) ctrlSpec {
	return ctrlSpec{
		style: wsChild | wsVisible | wsTabStop | wsVScroll | cbsDropDownList | cbsHasStrings,
		x:     x, y: y, cx: cx, cy: cy,
		id:    id,
		class: classComboBox,
	}
}

func passwordEdit(x, y, cx, cy int16, id uint16) ctrlSpec {
	return ctrlSpec{
		style: wsChild | wsVisible | wsTabStop | wsBorder | esPassword | esAutoHScroll,
		x:     x, y: y, cx: cx, cy: cy,
		id:    id,
		class: classEdit,
	}
}

func button(x, y, cx, cy int16, id uint16, text string, def bool) ctrlSpec {
	style := wsChild | wsVisible | wsTabStop
	if def {
		style |= bsDefPushButton
	} else {
		style |= bsPushButton
	}
	return ctrlSpec{
		style: style,
		x:     x, y: y, cx: cx, cy: cy,
		id:    id,
		class: classButton,
		text:  text,
	}
}

func writeUTF16(buf *bytes.Buffer, s string) {
	for _, c := range utf16.Encode([]rune(s)) {
		_ = binary.Write(buf, binary.LittleEndian, c)
	}
	_ = binary.Write(buf, binary.LittleEndian, uint16(0))
}

func alignDword(buf *bytes.Buffer) {
	for buf.Len()%4 != 0 {
		buf.WriteByte(0)
	}
}
