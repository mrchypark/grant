package adminui

import (
	"encoding/json"
	"fmt"
	"net/http"
	"net/url"

	"github.com/Conalog/grant/internal/grant"
)

func (s *server) loadRelated(r *http.Request, key, org, provider, selectedID string) ([]Table, error) {
	switch key {
	case "groups":
		if selectedID == "" {
			return nil, nil
		}
		data, _, err := s.api.call(r.Context(), http.MethodGet,
			"/api/v1/orgs/{organizationId}/groups/{groupId}/members",
			map[string]string{"organizationId": org, "groupId": selectedID}, nil, nil,
			org, s.cfg.ManagementOrganizationID)
		if err != nil {
			return nil, err
		}
		var members []grant.GroupMember
		if err := json.Unmarshal(data, &members); err != nil || members == nil {
			if err == nil {
				err = fmt.Errorf("expected an array")
			}
			return nil, fmt.Errorf("decode group members: %w", err)
		}
		rows := make([]Row, 0, len(members))
		for _, member := range members {
			row := Row{Cells: []string{member.UserID, member.OrganizationMembershipID}}
			if member.UserID != "" {
				row.Link = relatedLink("/members", org, provider, member.UserID, "")
			}
			rows = append(rows, row)
		}
		return []Table{{
			Title:   "그룹 구성원",
			Columns: []string{"사용자 ID", "조직 멤버십 ID"},
			Rows:    rows,
		}}, nil

	case "applications":
		data, _, err := s.api.call(r.Context(), http.MethodGet,
			"/api/v1/orgs/{organizationId}/app-grants",
			map[string]string{"organizationId": org}, nil, nil,
			org, s.cfg.ManagementOrganizationID)
		if err != nil {
			return nil, err
		}
		var grants []grant.ApplicationGrant
		if err := json.Unmarshal(data, &grants); err != nil || grants == nil {
			if err == nil {
				err = fmt.Errorf("expected an array")
			}
			return nil, fmt.Errorf("decode application grants: %w", err)
		}
		rows := make([]Row, 0, len(grants))
		for _, applicationGrant := range grants {
			if selectedID != "" && applicationGrant.ApplicationID != selectedID {
				continue
			}
			row := Row{Cells: []string{
				applicationGrant.ID,
				applicationGrant.UserID,
				applicationGrant.ApplicationID,
				applicationGrant.Status,
				applicationGrant.ConsentedAt,
			}}
			if applicationGrant.ID != "" && applicationGrant.ApplicationID != "" {
				row.Link = relatedLink("/applications", org, provider, applicationGrant.ApplicationID, applicationGrant.ID)
			}
			rows = append(rows, row)
		}
		return []Table{{
			Title:   "애플리케이션 권한 부여",
			Columns: []string{"부여 ID", "사용자 ID", "애플리케이션 ID", "상태", "동의 일시"},
			Rows:    rows,
		}}, nil
	default:
		return nil, nil
	}
}

func relatedLink(path, org, provider, id, grantID string) string {
	query := url.Values{"org": {org}, "id": {id}}
	if provider != "" {
		query.Set("provider", provider)
	}
	if grantID != "" {
		query.Set("grant", grantID)
	}
	return path + "?" + query.Encode()
}
