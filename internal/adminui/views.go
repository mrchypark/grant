package adminui

type Page struct {
	Title, Description, Section, OrganizationID, ProviderID, Search, CSRF, Notice, Error, Detail, NextURL string
	Navigation                                                                                            []NavItem
	Organizations, Providers                                                                              []Option
	Columns                                                                                               []string
	Rows                                                                                                  []Row
	Tables                                                                                                []Table
	Forms                                                                                                 []Form
}

type NavItem struct{ ID, Label, Group, Href string }
type Option struct{ Value, Label string }
type Row struct {
	Cells []string
	Link  string
}
type Table struct {
	Title   string
	Columns []string
	Rows    []Row
}
type Form struct {
	ID, Title, Description, Action, SubmitLabel, Confirmation string
	Fields                                                    []Field
	Hidden                                                    map[string]string
	Danger                                                    bool
}
type Field struct {
	Name, Label, Type, Value, Help string
	Required                       bool
	Options                        []Option
}
