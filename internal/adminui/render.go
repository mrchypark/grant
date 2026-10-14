package adminui

import (
	"bytes"
	"embed"
	"fmt"
	"html/template"
	"io"
	"net/http"
	"net/url"
	"strings"

	ui "github.com/mrchypark/ely-htmx"
)

//go:embed templates/*.html assets/*
var files embed.FS

type renderer struct{ templates *template.Template }

func newRenderer() (*renderer, error) {
	templates, err := ui.NewTemplates()
	if err != nil {
		return nil, fmt.Errorf("load Ely templates: %w", err)
	}
	templates.Funcs(template.FuncMap{
		"formHidden": formHidden,
		"queryHref":  queryHref,
		"sectionLabel": func(navigation []NavItem, section string) string {
			for _, item := range navigation {
				if item.ID == section {
					return item.Label
				}
			}
			return section
		},
		"elyOptions": func(options []Option) []ui.Option {
			converted := make([]ui.Option, len(options))
			for i, option := range options {
				converted[i] = ui.Option{Value: option.Value, Label: option.Label}
			}
			return converted
		},
	})
	if _, err := templates.ParseFS(files, "templates/*.html"); err != nil {
		return nil, fmt.Errorf("load admin templates: %w", err)
	}
	return &renderer{templates: templates}, nil
}

func (r *renderer) render(w io.Writer, p Page, partial bool) error {
	name := "page"
	if partial {
		name = "content"
	}
	var output bytes.Buffer
	if err := r.templates.ExecuteTemplate(&output, name, p); err != nil {
		return fmt.Errorf("render admin %s: %w", name, err)
	}
	_, err := output.WriteTo(w)
	return err
}

func bindAssets(mux *http.ServeMux) error {
	if err := ui.BindAssets(mux); err != nil {
		return fmt.Errorf("bind Ely assets: %w", err)
	}
	for _, asset := range []struct{ path, contentType string }{
		{"assets/grant-admin.css", "text/css; charset=utf-8"},
		{"assets/grant-admin.js", "text/javascript; charset=utf-8"},
	} {
		data, err := files.ReadFile(asset.path)
		if err != nil {
			return fmt.Errorf("read admin asset %s: %w", asset.path, err)
		}
		path, contentType := "/"+asset.path, asset.contentType
		mux.HandleFunc("GET "+path, func(w http.ResponseWriter, _ *http.Request) {
			w.Header().Set("Content-Type", contentType)
			w.Header().Set("X-Content-Type-Options", "nosniff")
			w.Header().Set("Cache-Control", "public, max-age=3600")
			_, _ = w.Write(data)
		})
	}
	return nil
}

// formHidden keeps backend-supplied operation context intact and makes the page CSRF token authoritative.
func formHidden(p Page, f Form) map[string]string {
	hidden := map[string]string{
		"section":  p.Section,
		"org":      p.OrganizationID,
		"provider": p.ProviderID,
	}
	for key, value := range f.Hidden {
		hidden[key] = value
	}
	hidden["csrf_token"] = p.CSRF
	return hidden
}

func queryHref(href string, p Page) string {
	u, err := url.Parse(href)
	if err != nil || u.IsAbs() || u.Host != "" || (u.Path != "" && !strings.HasPrefix(u.Path, "/")) {
		return href
	}
	q := u.Query()
	for key, value := range map[string]string{
		"org": p.OrganizationID, "provider": p.ProviderID, "q": p.Search,
	} {
		if value != "" && q.Get(key) == "" {
			q.Set(key, value)
		}
	}
	u.RawQuery = q.Encode()
	return u.String()
}
