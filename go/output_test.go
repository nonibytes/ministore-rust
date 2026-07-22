package ministorerust

import (
	"encoding/json"
	"testing"
	"time"
)

func outputSample() SearchResult {
	return SearchResult{
		Items: []json.RawMessage{
			json.RawMessage(`{"title":"Fast search","path":"/guides/search","priority":10,"category":"guide"}`),
			json.RawMessage(`{"path":"/notes/sqlite","title":"SQLite notes"}`),
		},
		NextCursor: "c:next",
		HasMore:    true,
	}
}

func TestFormatSearchResultsPretty(t *testing.T) {
	elapsed := 7 * time.Millisecond
	formatted, err := FormatSearchResults(outputSample(), SearchOutputOptions{Format: SearchOutputPretty, Elapsed: &elapsed})
	if err != nil {
		t.Fatal(err)
	}
	want := "Found 2 items in 7ms\n" +
		"- /guides/search\n" +
		"  category: guide\n" +
		"  priority: 10\n" +
		"  title: Fast search\n" +
		"- /notes/sqlite\n" +
		"  title: SQLite notes\n" +
		"\nNext cursor: c:next\n"
	if formatted != want {
		t.Fatalf("unexpected pretty output:\n%s", formatted)
	}
}

func TestFormatSearchResultsPaths(t *testing.T) {
	formatted, err := FormatSearchResults(outputSample(), SearchOutputOptions{Format: SearchOutputPaths})
	if err != nil {
		t.Fatal(err)
	}
	if formatted != "/guides/search\n/notes/sqlite\n" {
		t.Fatalf("unexpected paths output: %q", formatted)
	}
}

func TestFormatSearchResultsJSON(t *testing.T) {
	formatted, err := FormatSearchResults(outputSample(), SearchOutputOptions{Format: SearchOutputJSON})
	if err != nil {
		t.Fatal(err)
	}
	var decoded SearchResult
	if err := json.Unmarshal([]byte(formatted), &decoded); err != nil {
		t.Fatal(err)
	}
	if len(decoded.Items) != 2 || decoded.NextCursor != "c:next" || !decoded.HasMore {
		t.Fatalf("unexpected JSON result: %+v", decoded)
	}
}

func TestFormatSearchResultsRejectsUnknownFormat(t *testing.T) {
	if _, err := FormatSearchResults(SearchResult{}, SearchOutputOptions{Format: "yaml"}); err == nil {
		t.Fatal("expected invalid format error")
	}
}
