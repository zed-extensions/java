use tree_sitter::Query;

/// Zed refuses to load a language whose query names a node the grammar does not have, and the
/// file then renders with no highlighting at all. Every Gradle KTS query must compile against the
/// Kotlin grammar pinned in extension.toml.
#[test]
fn gradle_kts_queries_compile_against_pinned_kotlin_grammar() {
    let language = tree_sitter_kotlin::LANGUAGE.into();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("languages/gradle-kts");
    let mut checked = 0;
    for entry in std::fs::read_dir(&dir).expect("Failed to read languages/gradle-kts") {
        let path = entry.expect("Failed to read directory entry").path();
        if path.extension().is_some_and(|ext| ext == "scm") {
            let source = std::fs::read_to_string(&path).expect("Failed to read query");
            Query::new(&language, &source)
                .unwrap_or_else(|error| panic!("{} does not compile: {error}", path.display()));
            checked += 1;
        }
    }
    assert!(checked > 0, "No queries found in {}", dir.display());
}
