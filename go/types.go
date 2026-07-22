package ministorerust

import "encoding/json"

type FieldType string

const (
	FieldKeyword FieldType = "keyword"
	FieldText    FieldType = "text"
	FieldNumber  FieldType = "number"
	FieldDate    FieldType = "date"
	FieldBool    FieldType = "bool"
)

type FieldSpec struct {
	Type   FieldType `json:"type"`
	Multi  bool      `json:"multi,omitempty"`
	Weight *float64  `json:"weight,omitempty"`
}

type Schema struct {
	Fields map[string]FieldSpec `json:"fields"`
}

type SearchOptions struct {
	Limit      int      `json:"limit,omitempty"`
	After      string   `json:"after,omitempty"`
	Rank       string   `json:"rank,omitempty"`
	RankField  string   `json:"rank_field,omitempty"`
	Show       string   `json:"show,omitempty"`
	Fields     []string `json:"fields,omitempty"`
	Explain    bool     `json:"explain,omitempty"`
	CursorMode string   `json:"cursor_mode,omitempty"`
}

type SearchResult struct {
	Items        []json.RawMessage `json:"items"`
	NextCursor   string            `json:"next_cursor,omitempty"`
	HasMore      bool              `json:"has_more"`
	ExplainSQL   string            `json:"explain_sql,omitempty"`
	ExplainSteps []string          `json:"explain_steps,omitempty"`
}

type Item struct {
	Path        string          `json:"path"`
	Document    json.RawMessage `json:"document"`
	CreatedAtMS int64           `json:"created_at_ms"`
	UpdatedAtMS int64           `json:"updated_at_ms"`
}
