mod support;

// ============================================================================
// Main Method Tests
// ============================================================================

#[test]
fn runnables_multi_level_package() {
    support::assert_query_snapshot(
        "runnables_multi_level",
        "tests/languages/java/org/example/MultiLevelPackage.java",
        "languages/java/runnables.scm",
    );
}

#[test]
fn runnables_single_level_package() {
    support::assert_query_snapshot(
        "runnables_single_level",
        "tests/languages/java/example/SingleLevelPackage.java",
        "languages/java/runnables.scm",
    );
}

#[test]
fn runnables_default_package() {
    support::assert_query_snapshot(
        "runnables_default_package",
        "tests/languages/java/DefaultPackage.java",
        "languages/java/runnables.scm",
    );
}

// ============================================================================
// JUnit Test Tests
// ============================================================================

#[test]
fn runnables_multi_level_test() {
    support::assert_query_snapshot(
        "runnables_multi_level_test",
        "tests/languages/java/org/example/MultiLevelTest.java",
        "languages/java/runnables.scm",
    );
}

#[test]
fn runnables_single_level_test() {
    support::assert_query_snapshot(
        "runnables_single_level_test",
        "tests/languages/java/example/SingleLevelTest.java",
        "languages/java/runnables.scm",
    );
}

#[test]
fn runnables_default_package_test() {
    support::assert_query_snapshot(
        "runnables_default_package_test",
        "tests/languages/java/DefaultPackageTest.java",
        "languages/java/runnables.scm",
    );
}

#[test]
fn runnables_nested_test() {
    support::assert_query_snapshot(
        "runnables_nested_test",
        "tests/languages/java/example/NestedTest.java",
        "languages/java/runnables.scm",
    );
}
