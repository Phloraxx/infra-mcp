use globset::Glob;
use ignore::WalkBuilder;
use infra_protocol::{OperationResult, SearchMode, SearchRequest};
use regex::{Regex, RegexBuilder};
use serde::Serialize;
use std::{fs, path::Path};

const MAX_RESULTS: usize = 500;
const MAX_SEARCH_FILE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_CONTEXT_LINES: usize = 5;

#[derive(Debug, Serialize)]
struct SearchOutput {
    root: String,
    mode: &'static str,
    results: Vec<SearchHit>,
    truncated: bool,
    skipped_large_files: usize,
    skipped_binary_files: usize,
}

#[derive(Debug, Serialize)]
struct SearchHit {
    path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    line: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    column: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    snippet: Option<String>,
}

pub fn handle(request: SearchRequest) -> OperationResult {
    match search(request) {
        Ok(output) => match serde_json::to_value(output) {
            Ok(body) => OperationResult::Ok { body },
            Err(error) => OperationResult::Failed {
                error: error.to_string(),
            },
        },
        Err(error) => OperationResult::Failed { error },
    }
}

fn search(request: SearchRequest) -> Result<SearchOutput, String> {
    if request.query.is_empty() || request.query.len() > 4096 {
        return Err("query must be 1..4096 bytes".into());
    }
    let root = Path::new(&request.root);
    if !root.is_dir() {
        return Err("search root must be an existing directory".into());
    }
    let matcher = build_matcher(&request)?;
    let file_pattern = request
        .file_pattern
        .as_deref()
        .map(|pattern| {
            Glob::new(pattern)
                .map(|glob| glob.compile_matcher())
                .map_err(|error| format!("invalid file_pattern: {error}"))
        })
        .transpose()?;
    let max_results = request.max_results.unwrap_or(100).clamp(1, MAX_RESULTS);
    let context_lines = request.context_lines.unwrap_or(0).min(MAX_CONTEXT_LINES);
    let mut results = Vec::new();
    let mut truncated = false;
    let mut skipped_large_files = 0;
    let mut skipped_binary_files = 0;
    let mut walker = WalkBuilder::new(root);
    walker.hidden(!request.include_hidden);
    walker.git_ignore(true).git_global(true).git_exclude(true);

    'walk: for entry in walker.build().flatten() {
        let path = entry.path();
        if path == root || !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        let relative = path.strip_prefix(root).unwrap_or(path);
        if let Some(pattern) = &file_pattern {
            let filename_matches = path
                .file_name()
                .is_some_and(|name| pattern.is_match(Path::new(name)));
            if !pattern.is_match(relative) && !filename_matches {
                continue;
            }
        }

        match request.mode {
            SearchMode::Files => {
                let candidate = relative.to_string_lossy();
                if matcher.is_match(&candidate) {
                    results.push(SearchHit {
                        path: path.to_string_lossy().into_owned(),
                        line: None,
                        column: None,
                        snippet: None,
                    });
                    if results.len() >= max_results {
                        truncated = true;
                        break;
                    }
                }
            }
            SearchMode::Content => {
                let Ok(metadata) = entry.metadata() else {
                    continue;
                };
                if metadata.len() > MAX_SEARCH_FILE_BYTES {
                    skipped_large_files += 1;
                    continue;
                }
                let Ok(bytes) = fs::read(path) else {
                    continue;
                };
                if bytes.contains(&0) {
                    skipped_binary_files += 1;
                    continue;
                }
                let Ok(content) = std::str::from_utf8(&bytes) else {
                    skipped_binary_files += 1;
                    continue;
                };
                let lines = content.lines().collect::<Vec<_>>();
                for (index, line) in lines.iter().enumerate() {
                    let Some(found) = matcher.find(line) else {
                        continue;
                    };
                    let start = index.saturating_sub(context_lines);
                    let end = (index + context_lines + 1).min(lines.len());
                    results.push(SearchHit {
                        path: path.to_string_lossy().into_owned(),
                        line: Some(index + 1),
                        column: Some(found.start() + 1),
                        snippet: Some(lines[start..end].join("\n")),
                    });
                    if results.len() >= max_results {
                        truncated = true;
                        break 'walk;
                    }
                }
            }
        }
    }

    Ok(SearchOutput {
        root: request.root,
        mode: match request.mode {
            SearchMode::Files => "files",
            SearchMode::Content => "content",
        },
        results,
        truncated,
        skipped_large_files,
        skipped_binary_files,
    })
}

struct Matcher(Regex);
impl Matcher {
    fn is_match(&self, value: &str) -> bool {
        self.0.is_match(value)
    }
    fn find<'a>(&self, value: &'a str) -> Option<regex::Match<'a>> {
        self.0.find(value)
    }
}

fn build_matcher(request: &SearchRequest) -> Result<Matcher, String> {
    let expression = if request.literal {
        regex::escape(&request.query)
    } else {
        request.query.clone()
    };
    RegexBuilder::new(&expression)
        .case_insensitive(request.ignore_case)
        .build()
        .map(Matcher)
        .map_err(|error| format!("invalid search expression: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn content_search_returns_line_and_context() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("a.rs"), "one\ntwo target\nthree\n").unwrap();
        let output = search(SearchRequest {
            root: dir.path().to_string_lossy().into_owned(),
            query: "target".into(),
            mode: SearchMode::Content,
            literal: true,
            ignore_case: false,
            include_hidden: false,
            file_pattern: Some("*.rs".into()),
            max_results: Some(10),
            context_lines: Some(1),
        })
        .unwrap();
        assert_eq!(output.results.len(), 1);
        assert_eq!(output.results[0].line, Some(2));
        assert_eq!(output.results[0].column, Some(5));
        assert_eq!(
            output.results[0].snippet.as_deref(),
            Some("one\ntwo target\nthree")
        );
    }

    #[test]
    fn filename_search_respects_hidden_default() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("visible.txt"), "ok").unwrap();
        fs::write(dir.path().join(".secret.txt"), "hidden").unwrap();
        let output = search(SearchRequest {
            root: dir.path().to_string_lossy().into_owned(),
            query: "txt".into(),
            mode: SearchMode::Files,
            literal: true,
            ignore_case: true,
            include_hidden: false,
            file_pattern: None,
            max_results: Some(10),
            context_lines: None,
        })
        .unwrap();
        assert_eq!(output.results.len(), 1);
        assert!(output.results[0].path.ends_with("visible.txt"));
    }
}
