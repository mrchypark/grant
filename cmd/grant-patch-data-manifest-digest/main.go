package main

import (
	"bytes"
	"encoding/json"
	"fmt"
	"github.com/Conalog/grant/internal/grant"
	"io"
	"os"
	"reflect"
	"strings"
)

func main() {
	if e := run(os.Stdin, os.Stdout); e != nil {
		fmt.Fprintln(os.Stderr, e)
		os.Exit(1)
	}
}
func run(in io.Reader, out io.Writer) error {
	raw, e := io.ReadAll(in)
	if e != nil {
		return e
	}
	var object map[string]json.RawMessage
	if e = json.Unmarshal(raw, &object); e != nil {
		return e
	}
	if object == nil {
		return fmt.Errorf("manifest must be a JSON object")
	}
	object["manifestDigest"] = json.RawMessage(`"` + strings.Repeat("0", 64) + `"`)
	raw, e = json.Marshal(object)
	if e != nil {
		return e
	}
	var manifest grant.PatchDataAuthorityImport
	if e = requiredShape(raw, reflect.TypeFor[grant.PatchDataAuthorityImport]()); e != nil {
		return e
	}
	decoder := json.NewDecoder(bytes.NewReader(raw))
	decoder.DisallowUnknownFields()
	if e = decoder.Decode(&manifest); e != nil {
		return e
	}
	if e = manifestEnums(manifest); e != nil {
		return e
	}
	_, e = fmt.Fprintln(out, grant.PatchDataManifestDigest(manifest))
	return e
}

func manifestEnums(m grant.PatchDataAuthorityImport) error {
	valid := func(value string, allowed ...string) bool {
		for _, a := range allowed {
			if value == a {
				return true
			}
		}
		return false
	}
	for _, v := range m.Memberships {
		if !valid(v.Role, "owner", "admin", "member") || !valid(v.Status, "active", "disabled") {
			return fmt.Errorf("invalid membership role or status")
		}
	}
	for _, v := range m.PlantBindings {
		if !valid(v.SubjectType, "organization", "group", "user") || !valid(v.State, "active", "disabled") {
			return fmt.Errorf("invalid binding subjectType or state")
		}
	}
	return nil
}

// serde's typed manifest rejects absent/non-null scalar and collection fields.
// Validate the shape before Go's decoder can replace them with zero values.
func requiredShape(raw json.RawMessage, t reflect.Type) error {
	if t.Kind() == reflect.Pointer {
		if bytes.Equal(bytes.TrimSpace(raw), []byte("null")) {
			return nil
		}
		return requiredShape(raw, t.Elem())
	}
	if bytes.Equal(bytes.TrimSpace(raw), []byte("null")) {
		return fmt.Errorf("required manifest field must not be null")
	}
	switch t.Kind() {
	case reflect.Struct:
		var fields map[string]json.RawMessage
		if e := json.Unmarshal(raw, &fields); e != nil {
			return e
		}
		if fields == nil {
			return fmt.Errorf("manifest must be a JSON object")
		}
		for i := 0; i < t.NumField(); i++ {
			f := t.Field(i)
			name := strings.Split(f.Tag.Get("json"), ",")[0]
			v, ok := fields[name]
			if !ok {
				if f.Type.Kind() == reflect.Pointer {
					continue
				}
				return fmt.Errorf("missing field %s", name)
			}
			if e := requiredShape(v, f.Type); e != nil {
				return fmt.Errorf("%s: %w", name, e)
			}
		}
	case reflect.Slice:
		var values []json.RawMessage
		if e := json.Unmarshal(raw, &values); e != nil {
			return e
		}
		for _, v := range values {
			if e := requiredShape(v, t.Elem()); e != nil {
				return e
			}
		}
	}
	return nil
}
