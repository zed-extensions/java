use std::path::Path;
use tree_sitter::{Parser, Query, QueryCursor, StreamingIterator};

/// Represents a single capture from a query match
#[derive(Debug, serde::Serialize)]
pub struct Capture {
    pub name: String,
    pub line: usize,
    pub column: usize,
    pub text: String,
}

fn normalize_line_endings(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

pub fn run_query(source: &str, query_source: &str) -> Vec<Capture> {
    let mut parser = Parser::new();
    let language = tree_sitter_java::LANGUAGE.into();
    parser
        .set_language(&language)
        .unwrap_or_else(|_| panic!("Error loading Java parser"));

    let tree = parser.parse(source, None).expect("Failed to parse source");
    let query = Query::new(&language, query_source).expect("Failed to create query");

    let mut cursor = QueryCursor::new();
    let mut captures = Vec::new();
    let source_bytes = source.as_bytes();

    let mut matches = cursor.matches(&query, tree.root_node(), source_bytes);
    while let Some(match_) = matches.next() {
        for capture in match_.captures {
            let capture_name = &query.capture_names()[capture.index as usize];
            let node = capture.node;
            let start = node.start_position();
            let text = node
                .utf8_text(source_bytes)
                .expect("Failed to extract capture text");

            captures.push(Capture {
                name: capture_name.to_string(),
                line: start.row + 1,
                column: start.column + 1,
                text: text.to_string(),
            });
        }
    }

    captures
}

pub fn assert_query_snapshot(snapshot_name: &str, fixture_path: &str, query_path: &str) {
    let fixture_abs = Path::new(env!("CARGO_MANIFEST_DIR")).join(fixture_path);
    let query_abs = Path::new(env!("CARGO_MANIFEST_DIR")).join(query_path);
    let source = std::fs::read_to_string(&fixture_abs).unwrap_or_else(|error| {
        panic!(
            "Failed to read query fixture file {}: {error}",
            fixture_abs.display()
        )
    });
    let query_source = std::fs::read_to_string(&query_abs).unwrap_or_else(|error| {
        panic!("Failed to read query file {}: {error}", query_abs.display())
    });

    let captures = run_query(
        &normalize_line_endings(&source),
        &normalize_line_endings(&query_source),
    );

    let snapshot_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("languages")
        .join("java")
        .join("snapshots");
    std::fs::create_dir_all(&snapshot_dir).expect("Failed to create snapshot directory");

    let mut settings = insta::Settings::clone_current();
    settings.set_snapshot_path(snapshot_dir);
    settings.set_prepend_module_to_snapshot(false);
    settings.bind(|| {
        insta::assert_yaml_snapshot!(snapshot_name, captures);
    });
}
