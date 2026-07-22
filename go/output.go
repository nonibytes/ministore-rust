package ministorerust

import (
	"bytes"
	"encoding/json"
	"fmt"
	"sort"
	"strings"
	"time"
)

type SearchOutputFormat string

const (
	SearchOutputPretty SearchOutputFormat = "pretty"
	SearchOutputPaths  SearchOutputFormat = "paths"
	SearchOutputJSON   SearchOutputFormat = "json"
)

type SearchOutputOptions struct {
	Format  SearchOutputFormat
	Elapsed *time.Duration
}

func ParseSearchOutputFormat(value string) (SearchOutputFormat, error) {
	switch SearchOutputFormat(strings.ToLower(value)) {
	case "", SearchOutputPretty:
		return SearchOutputPretty, nil
	case SearchOutputPaths:
		return SearchOutputPaths, nil
	case SearchOutputJSON:
		return SearchOutputJSON, nil
	default:
		return "", fmt.Errorf("unknown search output format %q; expected pretty, paths, or json", value)
	}
}

// FormatSearchResults renders a search page for terminal or machine output.
// Pretty and paths are human-readable; JSON has a stable page envelope.
func FormatSearchResults(result SearchResult, options SearchOutputOptions) (string, error) {
	format, err := ParseSearchOutputFormat(string(options.Format))
	if err != nil {
		return "", err
	}
	switch format {
	case SearchOutputPretty:
		return formatSearchPretty(result, options.Elapsed)
	case SearchOutputPaths:
		return formatSearchPaths(result)
	case SearchOutputJSON:
		encoded, err := json.MarshalIndent(result, "", "  ")
		if err != nil {
			return "", fmt.Errorf("format search results as JSON: %w", err)
		}
		return string(encoded) + "\n", nil
	default:
		panic("unreachable search output format")
	}
}

func formatSearchPretty(result SearchResult, elapsed *time.Duration) (string, error) {
	var output strings.Builder
	fmt.Fprintf(&output, "Found %d items", len(result.Items))
	if elapsed != nil {
		fmt.Fprintf(&output, " in %dms", elapsed.Milliseconds())
	}
	output.WriteByte('\n')
	for index, rawItem := range result.Items {
		item, err := decodeOutputItem(rawItem, index)
		if err != nil {
			return "", err
		}
		path := "(no path)"
		if rawPath, ok := item["path"]; ok {
			if err := json.Unmarshal(rawPath, &path); err != nil {
				return "", fmt.Errorf("format search result %d: path must be a string: %w", index, err)
			}
		}
		fmt.Fprintf(&output, "- %s\n", path)
		keys := make([]string, 0, len(item))
		for key := range item {
			if key != "path" {
				keys = append(keys, key)
			}
		}
		sort.Strings(keys)
		for _, key := range keys {
			value, err := formatOutputValue(item[key])
			if err != nil {
				return "", fmt.Errorf("format search result %d field %q: %w", index, key, err)
			}
			fmt.Fprintf(&output, "  %s: %s\n", key, value)
		}
	}
	if result.NextCursor != "" {
		fmt.Fprintf(&output, "\nNext cursor: %s\n", result.NextCursor)
	} else if result.HasMore {
		output.WriteString("\nMore results available\n")
	}
	if len(result.ExplainSteps) > 0 {
		output.WriteString("\nExplanation:\n")
		for _, step := range result.ExplainSteps {
			fmt.Fprintf(&output, "  %s\n", step)
		}
	}
	if result.ExplainSQL != "" {
		fmt.Fprintf(&output, "\nSQL: %s\n", result.ExplainSQL)
	}
	return output.String(), nil
}

func formatSearchPaths(result SearchResult) (string, error) {
	var output strings.Builder
	for index, rawItem := range result.Items {
		item, err := decodeOutputItem(rawItem, index)
		if err != nil {
			return "", err
		}
		rawPath, ok := item["path"]
		if !ok {
			continue
		}
		var path string
		if err := json.Unmarshal(rawPath, &path); err != nil {
			return "", fmt.Errorf("format search result %d: path must be a string: %w", index, err)
		}
		output.WriteString(path)
		output.WriteByte('\n')
	}
	return output.String(), nil
}

func decodeOutputItem(raw json.RawMessage, index int) (map[string]json.RawMessage, error) {
	var item map[string]json.RawMessage
	if err := json.Unmarshal(raw, &item); err != nil {
		return nil, fmt.Errorf("format search result %d: invalid JSON object: %w", index, err)
	}
	if item == nil {
		return nil, fmt.Errorf("format search result %d: expected JSON object", index)
	}
	return item, nil
}

func formatOutputValue(raw json.RawMessage) (string, error) {
	if len(raw) > 0 && raw[0] == '"' {
		var value string
		if err := json.Unmarshal(raw, &value); err != nil {
			return "", err
		}
		return value, nil
	}
	var compact bytes.Buffer
	if err := json.Compact(&compact, raw); err != nil {
		return "", err
	}
	return compact.String(), nil
}
