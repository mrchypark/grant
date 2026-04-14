//go:build !darwin

package localization

import "os"

func SystemLanguage() Language {
	return languageFromTag(os.Getenv("LANG"))
}
