mod support;

#[test]
fn runnables_multi_level_package() {
    support::assert_query_snapshot(
        "runnables_multi_level",
        "tests/languages/java/org/example/Main.java",
        "languages/java/runnables.scm",
    );
}

#[test]
fn runnables_single_level_package() {
    support::assert_query_snapshot(
        "runnables_single_level",
        "tests/languages/java/example/Main.java",
        "languages/java/runnables.scm",
    );
}
