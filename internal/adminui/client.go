package adminui

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/url"
	"strings"
	"time"

	"github.com/Conalog/grant/internal/adminauth"
	"github.com/Conalog/grant/internal/grant"
)

const maxAPIResponse = 4 << 20

type apiClient struct {
	base   *url.URL
	client *http.Client
}

type apiResponse struct {
	body   []byte
	status int
	etag   string
}

func newAPIClient(raw string, supplied *http.Client) (*apiClient, error) {
	u, err := url.Parse(strings.TrimSpace(raw))
	if err != nil || u == nil || !u.IsAbs() || u.Host == "" || u.User != nil || u.Opaque != "" || (u.Scheme != "https" && u.Scheme != "http") || u.RawQuery != "" || u.ForceQuery || u.Fragment != "" {
		return nil, errors.New("API URL must be an absolute HTTP URL without credentials, query, or fragment")
	}
	if u.Scheme != "https" && !isLoopbackHost(u.Hostname()) {
		return nil, errors.New("API URL must use HTTPS unless it is loopback")
	}
	c := http.Client{Timeout: 12 * time.Second}
	if supplied != nil {
		c = *supplied
		c.Timeout = 12 * time.Second
	}
	if c.Transport == nil {
		t := http.DefaultTransport.(*http.Transport).Clone()
		t.Proxy = nil
		c.Transport = t
	} else if t, ok := c.Transport.(*http.Transport); ok {
		t = t.Clone()
		t.Proxy = nil
		c.Transport = t
	}
	c.CheckRedirect = func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }
	return &apiClient{base: u, client: &c}, nil
}

func isLoopbackHost(host string) bool {
	if strings.EqualFold(host, "localhost") {
		return true
	}
	ip := net.ParseIP(host)
	return ip != nil && ip.IsLoopback()
}

func validID(id string) bool {
	return id != "" && id != "." && id != ".." && !strings.ContainsAny(id, "/\\\x00")
}

// call resolves only routes in Grant's canonical management registry.
func (c *apiClient) call(ctx context.Context, method, route string, vars map[string]string, query url.Values, body any, organization, managementOrganization string, extra ...http.Header) ([]byte, int, error) {
	response, err := c.callWithMeta(ctx, method, route, vars, query, body, organization, managementOrganization, extra...)
	return response.body, response.status, err
}

func (c *apiClient) callWithMeta(ctx context.Context, method, route string, vars map[string]string, query url.Values, body any, organization, managementOrganization string, extra ...http.Header) (apiResponse, error) {
	definition, ok := grant.LookupManagementRoute(method, route)
	if !ok {
		return apiResponse{}, errors.New("unsupported management operation")
	}
	path, err := expandManagementPath(route, vars)
	if err != nil {
		return apiResponse{}, err
	}
	u := *c.base
	basePath := strings.TrimRight(u.EscapedPath(), "/")
	escapedPath := basePath + path
	u.Path, _ = url.PathUnescape(escapedPath)
	u.RawPath = escapedPath
	u.RawQuery = query.Encode()
	var payload io.Reader
	if body != nil {
		b, err := json.Marshal(body)
		if err != nil {
			return apiResponse{}, err
		}
		payload = bytes.NewReader(b)
	}
	req, err := http.NewRequestWithContext(ctx, method, u.String(), payload)
	if err != nil {
		return apiResponse{}, errors.New("invalid management request")
	}
	token := adminauth.AccessToken(ctx)
	if token == "" {
		return apiResponse{}, errors.New("authenticated session required")
	}
	req.Header.Set("Authorization", "Bearer "+token)
	req.Header.Set("Accept", "application/json")
	if body != nil {
		req.Header.Set("Content-Type", "application/json")
	}
	org := organization
	if definition.Platform || (org == "" && definition.ActionCode == "provider.list") {
		org = managementOrganization
	}
	if org != "" {
		req.Header.Set("X-Organization-ID", org)
	}
	if len(extra) > 0 {
		for key, values := range extra[0] {
			for _, value := range values {
				req.Header.Add(key, value)
			}
		}
	}
	resp, err := c.client.Do(req)
	if err != nil {
		return apiResponse{}, errors.New("Grant API is unavailable")
	}
	defer resp.Body.Close()
	b, err := io.ReadAll(io.LimitReader(resp.Body, maxAPIResponse+1))
	if err != nil || len(b) > maxAPIResponse {
		return apiResponse{}, errors.New("Grant API response was invalid or too large")
	}
	result := apiResponse{body: b, status: resp.StatusCode, etag: resp.Header.Get("ETag")}
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		return result, errors.New(safeAPIError(b, resp.StatusCode))
	}
	return result, nil
}

func expandManagementPath(route string, vars map[string]string) (string, error) {
	path := route
	for key, value := range vars {
		placeholder := "{" + key + "}"
		if !strings.Contains(path, placeholder) {
			continue
		}
		if !validID(value) {
			return "", fmt.Errorf("invalid %s", key)
		}
		path = strings.ReplaceAll(path, placeholder, url.PathEscape(value))
	}
	if strings.Contains(path, "{") || strings.Contains(path, "}") {
		return "", errors.New("missing route identifier")
	}
	return path, nil
}

func safeAPIError(body []byte, status int) string {
	var payload struct {
		Error string `json:"error"`
	}
	message := "Grant API request failed"
	if json.Unmarshal(body, &payload) == nil && payload.Error != "" && len(payload.Error) <= 500 && !strings.ContainsAny(payload.Error, "\r\n") {
		message = payload.Error
	}
	return fmt.Sprintf("%s (HTTP %d)", message, status)
}
