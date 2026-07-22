use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

fn setup_mock_project(
    temp_dir: &Path,
    project_type: &str,
    module_path: Option<&str>,
) -> (PathBuf, PathBuf, PathBuf) {
    let module_dir = if let Some(path) = module_path {
        let d = temp_dir.join(path);
        fs::create_dir_all(&d).unwrap();
        d
    } else {
        temp_dir.to_path_buf()
    };

    let main_package_dir = module_dir.join("src/main/java/com/example");
    let test_package_dir = module_dir.join("src/test/java/com/example");

    fs::create_dir_all(&main_package_dir).unwrap();
    fs::create_dir_all(&test_package_dir).unwrap();

    let bin_dir = temp_dir.join("bin");
    fs::create_dir_all(&bin_dir).unwrap();

    if project_type == "maven" {
        fs::File::create(temp_dir.join("pom.xml")).unwrap();
        if let Some(path) = module_path {
            fs::File::create(temp_dir.join(path).join("pom.xml")).unwrap();
        }
        let mvn_mock = bin_dir.join("mvn");
        fs::write(&mvn_mock, "#!/bin/sh\necho \"MVN_CALLED: $@\"").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&mvn_mock, fs::Permissions::from_mode(0o755)).unwrap();
        }
    } else if project_type == "gradle" {
        fs::File::create(temp_dir.join("settings.gradle")).unwrap();
        if let Some(path) = module_path {
            fs::File::create(temp_dir.join(path).join("build.gradle")).unwrap();
        }
        let gradle_mock = bin_dir.join("gradle");
        fs::write(&gradle_mock, "#!/bin/sh\necho \"GRADLE_CALLED: $@\"").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&gradle_mock, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    let zed_file = main_package_dir.join("Main.java");
    let zed_test_file = test_package_dir.join("MainTest.java");
    fs::File::create(&zed_file).unwrap();
    fs::File::create(&zed_test_file).unwrap();

    (zed_file, zed_test_file, bin_dir)
}
fn get_task_command_by_tag(tag: &str) -> String {
    let tasks_json =
        fs::read_to_string("languages/java/tasks.json").expect("Failed to read tasks.json");
    let tasks: Value = serde_json::from_str(&tasks_json).expect("Failed to parse tasks.json");
    let tasks_array = tasks.as_array().expect("tasks.json is not an array");

    for task in tasks_array {
        if let Some(tags) = task["tags"].as_array()
            && tags.iter().any(|t| t.as_str() == Some(tag))
        {
            return task["command"]
                .as_str()
                .expect("Command is not a string")
                .to_string();
        }
    }
    panic!("Task with tag '{}' not found", tag);
}

struct TestProject {
    temp_dir: PathBuf,
    bin_dir: PathBuf,
    zed_file: PathBuf,
    zed_test_file: PathBuf,
    new_path: String,
}

impl TestProject {
    fn new(name: &str, project_type: &str, module_path: Option<&str>) -> Self {
        let temp_dir = std::env::temp_dir().join(name);
        if temp_dir.exists() {
            fs::remove_dir_all(&temp_dir).unwrap();
        }
        fs::create_dir_all(&temp_dir).unwrap();
        let (zed_file, zed_test_file, bin_dir) =
            setup_mock_project(&temp_dir, project_type, module_path);
        let old_path = std::env::var("PATH").unwrap_or_default();
        let new_path = format!("{}:{}", bin_dir.to_string_lossy(), old_path);
        Self {
            temp_dir,
            bin_dir,
            zed_file,
            zed_test_file,
            new_path,
        }
    }

    fn task(&self, tag: &str) -> TaskRunner<'_> {
        let command = get_task_command_by_tag(tag).replace(
            "${ZED_CUSTOM_java_outer_class_name:}",
            "${ZED_CUSTOM_java_outer_class_name:-}",
        );
        TaskRunner {
            project: self,
            command,
            zed_file: self.zed_file.clone(),
            package: "com.example".to_string(),
            class: "Main".to_string(),
            extra_env: Vec::new(),
        }
    }

    fn mock_bin(&self, name: &str, content: &str) {
        let bin_path = self.bin_dir.join(name);
        fs::write(&bin_path, content).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&bin_path, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
}

impl Drop for TestProject {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.temp_dir);
    }
}

static BUILD_HELPER: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

fn ensure_task_helper_built() -> PathBuf {
    BUILD_HELPER
        .get_or_init(|| {
            let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|_| std::env::current_dir().unwrap());

            let mut cmd = std::process::Command::new("cargo");
            cmd.current_dir(&manifest_dir);
            cmd.env_remove("CARGO_BUILD_TARGET");
            cmd.args([
                "build",
                "-p",
                "java-task-helper",
                "--bin",
                "java-task-helper",
                "--message-format=json",
            ]);
            if !cfg!(debug_assertions) {
                cmd.arg("--release");
            }

            let output = cmd
                .output()
                .expect("failed to run cargo build for java-task-helper");
            assert!(
                output.status.success(),
                "cargo build -p java-task-helper failed:\n{}",
                String::from_utf8_lossy(&output.stderr)
            );

            let stdout = String::from_utf8_lossy(&output.stdout);
            let mut found_path = None;
            for line in stdout.lines() {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
                    let is_target_bin = val["target"]["kind"]
                        .as_array()
                        .is_some_and(|k| k.iter().any(|kind| kind == "bin"));
                    if val["reason"] == "compiler-artifact"
                        && val["target"]["name"] == "java-task-helper"
                        && is_target_bin
                    {
                        found_path = val["filenames"]
                            .as_array()
                            .and_then(|files| files.first())
                            .and_then(|f| f.as_str())
                            .map(PathBuf::from);
                    }
                }
            }

            found_path.expect(
                "Could not find compiler artifact filename for java-task-helper in cargo build output",
            )
        })
        .clone()
}

struct TaskRunner<'a> {
    project: &'a TestProject,
    command: String,
    zed_file: PathBuf,
    package: String,
    class: String,
    extra_env: Vec<(&'static str, String)>,
}

impl<'a> TaskRunner<'a> {
    fn zed_file(mut self, path: PathBuf) -> Self {
        self.zed_file = path;
        self
    }
    fn package(mut self, p: &str) -> Self {
        self.package = p.to_string();
        self
    }
    fn class(mut self, c: &str) -> Self {
        self.class = c.to_string();
        self
    }
    fn method(mut self, m: &str) -> Self {
        self.extra_env
            .push(("ZED_CUSTOM_java_method_name", m.to_string()));
        self
    }
    fn outer_class(mut self, o: &str) -> Self {
        self.extra_env
            .push(("ZED_CUSTOM_java_outer_class_name", o.to_string()));
        self
    }
    fn env(mut self, key: &'static str, value: &str) -> Self {
        self.extra_env.push((key, value.to_string()));
        self
    }

    fn run(self) -> String {
        let mut cmd = std::process::Command::new("sh");

        let found_bin = ensure_task_helper_built();

        // Create a temporary ZED_EXT directory structure matching what tasks.json expects
        let zed_ext_base = self.project.temp_dir.join("mock_zed_ext");
        let zed_ext_dir = if cfg!(target_os = "macos") {
            zed_ext_base.join("Library/Application Support/Zed/extensions/work/java/task-bin")
        } else {
            zed_ext_base.join("zed/extensions/work/java/task-bin")
        };
        fs::create_dir_all(&zed_ext_dir).unwrap();
        let dest_bin = zed_ext_dir.join("java-task-helper");
        fs::copy(&found_bin, &dest_bin).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&dest_bin, fs::Permissions::from_mode(0o755)).unwrap();
        }

        cmd.arg("-c")
            .arg(&self.command)
            .env("ZED_FILE", self.zed_file.to_string_lossy().to_string())
            .env("PWD", self.project.temp_dir.to_string_lossy().to_string())
            .env("ZED_CUSTOM_java_package_name", &self.package)
            .env("ZED_CUSTOM_java_class_name", &self.class)
            .env("PATH", &self.project.new_path)
            .env("HOME", &zed_ext_base)
            .env("XDG_DATA_HOME", zed_ext_base.to_string_lossy().to_string())
            .current_dir(&self.project.temp_dir);

        for (k, v) in self.extra_env {
            cmd.env(k, v);
        }

        let output = cmd.output().expect("Failed to execute shell command");
        if !output.status.success() {
            panic!(
                "Shell command failed with status: {:?}\nStdout: {}\nStderr: {}",
                output.status,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        String::from_utf8_lossy(&output.stdout).to_string()
    }
}

// --- Maven Tests ---

#[test]
fn test_maven_single_module_command_logic() {
    let project = TestProject::new("maven_single", "maven", None);

    let stdout = project.task("java-main").run();
    let stdout_test = project
        .task("java-main")
        .zed_file(project.zed_test_file.clone())
        .run();

    assert!(
        stdout.contains("MVN_CALLED: compile exec:exec"),
        "Should run as single module. Got: {}",
        stdout
    );
    assert!(
        stdout.contains("-Dexec.executable=java")
            && stdout.contains("-Dexec.args=-classpath %classpath com.example.Main")
            && stdout.contains("-Dexec.inheritIo=true")
            && stdout.contains("-Dexec.longClasspath=true"),
        "Should launch the main class in a standalone JVM. Got: {}",
        stdout
    );
    assert!(
        stdout.contains("-Dexec.classpathScope=runtime"),
        "Should use runtime classpath scope. Got: {}",
        stdout
    );
    assert!(
        stdout_test.contains("MVN_CALLED: test-compile exec:exec")
            && stdout_test.contains("-Dexec.args=-classpath %classpath com.example.Main"),
        "Should run as single module. Got: {}",
        stdout_test
    );
    assert!(
        stdout_test.contains("-Dexec.classpathScope=test"),
        "Should use test classpath scope. Got: {}",
        stdout_test
    );
}

#[test]
fn test_maven_multi_module_command_logic() {
    let project = TestProject::new("maven_multi", "maven", Some("module-a"));

    let stdout = project.task("java-main").run();
    let stdout_test = project
        .task("java-main")
        .zed_file(project.zed_test_file.clone())
        .run();

    assert!(
        stdout.contains("MVN_CALLED: compile exec:exec -pl module-a -am"),
        "Should build submodule with dependencies. Got: {}",
        stdout
    );
    assert!(
        stdout.contains("-Dexec.args=-classpath %classpath com.example.Main"),
        "Should run only the submodule. Got: {}",
        stdout
    );
    assert!(
        stdout.contains("-Dexec.classpathScope=runtime"),
        "Should use runtime classpath scope. Got: {}",
        stdout
    );

    assert!(
        stdout_test.contains("MVN_CALLED: test-compile exec:exec -pl module-a -am"),
        "Should build submodule with dependencies. Got: {}",
        stdout_test
    );
    assert!(
        stdout_test.contains("-Dexec.args=-classpath %classpath com.example.Main"),
        "Should run only the submodule. Got: {}",
        stdout_test
    );
    assert!(
        stdout_test.contains("-Dexec.classpathScope=test"),
        "Should use test classpath scope. Got: {}",
        stdout_test
    );
}

#[test]
fn test_maven_nested_module_command_logic() {
    let project = TestProject::new("maven_nested", "maven", Some("nested/module-b"));

    let stdout = project
        .task("java-main")
        .zed_file(project.zed_test_file.clone())
        .run();

    assert!(
        stdout.contains("MVN_CALLED: test-compile exec:exec -pl nested/module-b -am"),
        "Should build nested submodule with dependencies. Got: {}",
        stdout
    );
    assert!(
        stdout.contains("-Dexec.args=-classpath %classpath com.example.Main"),
        "Should run only the nested submodule. Got: {}",
        stdout
    );
}

#[test]
fn test_maven_multi_module_test_method_logic() {
    let project = TestProject::new("maven_method", "maven", Some("module-a"));

    let stdout = project
        .task("java-test-method")
        .method("shouldPersist")
        .run();

    assert!(
        stdout.contains("MVN_CALLED: test")
            && stdout.contains("-U")
            && stdout.contains("-pl module-a -am"),
        "Should build submodule with dependencies. Got: {}",
        stdout
    );
    assert!(
        stdout.contains("-Dtest=com.example.Main#shouldPersist"),
        "Should run only the submodule test. Got: {}",
        stdout
    );
}

#[test]
fn test_maven_nested_class_test_method_logic() {
    let project = TestProject::new("maven_nested_class", "maven", Some("module-a"));

    let stdout = project
        .task("java-test-method")
        .class("Inner")
        .outer_class("Outer")
        .method("testMe")
        .run();

    assert!(
        stdout.contains("-Dtest=com.example.Outer$Inner#testMe"),
        "Should run nested class test method correctly. Got: {}",
        stdout
    );
}

#[test]
fn test_maven_run_all_tests_logic() {
    let project = TestProject::new("maven_all_tests", "maven", Some("module-a"));

    let stdout = project.task("java-test-all").run();

    assert!(
        stdout.contains("MVN_CALLED: test")
            && stdout.contains("-U")
            && stdout.contains("-pl module-a -am"),
        "Should build submodule with dependencies. Got: {}",
        stdout
    );
    assert!(
        stdout.contains("-pl module-a -am"),
        "Should run all tests in submodule. Got: {}",
        stdout
    );
}

#[test]
fn test_maven_test_class_logic() {
    let project = TestProject::new("maven_test_class", "maven", Some("module-a"));

    let stdout = project.task("java-test-class").run();

    assert!(
        stdout.contains("MVN_CALLED: test")
            && stdout.contains("-U")
            && stdout.contains("-pl module-a -am"),
        "Should build submodule with dependencies. Got: {}",
        stdout
    );
    assert!(
        stdout.contains("-Dtest=com.example.Main"),
        "Should run only the submodule test class. Got: {}",
        stdout
    );
}

#[test]
fn test_maven_single_level_package_logic() {
    let project = TestProject::new("maven_single_package", "maven", None);
    let stdout = project
        .task("java-main")
        .package("example")
        .class("Main")
        .run();

    assert!(
        stdout.contains("-Dexec.args=-classpath %classpath example.Main"),
        "Should include the single-level package in Maven. Got: {}",
        stdout
    );
}

#[test]
fn test_maven_default_package_command_logic() {
    let project = TestProject::new("maven_default_package", "maven", None);
    let stdout = project.task("java-main").package("").class("Main").run();

    assert!(
        stdout.contains("-Dexec.args=-classpath %classpath Main"),
        "Should not include leading dot for default package in Maven. Got: {}",
        stdout
    );
}

#[test]
fn test_maven_debugs_standalone_jvm() {
    let project = TestProject::new("maven_debug", "maven", None);
    let stdout = project
        .task("java-main")
        .env("ZED_JAVA_DEBUG", "1")
        .env("ZED_JAVA_DEBUG_PORT", "6123")
        .run();

    assert!(
        stdout.contains(
            "-Dexec.args=-agentlib:jdwp=transport=dt_socket,server=y,suspend=y,address=6123 \
             -classpath %classpath com.example.Main"
        ),
        "Should pass JDWP options to the standalone JVM. Got: {}",
        stdout
    );
    assert!(
        !stdout.contains("MAVEN_OPTS"),
        "Should not debug the Maven process. Got: {}",
        stdout
    );
}

#[test]
fn test_maven_default_package_test_method_logic() {
    let project = TestProject::new("maven_default_test", "maven", None);
    let stdout = project
        .task("java-test-method")
        .package("")
        .class("MyTest")
        .method("testMethod")
        .run();

    assert!(
        stdout.contains("-Dtest=MyTest#testMethod"),
        "Should not include leading dot for test method in default package. Got: {}",
        stdout
    );
}

#[test]
fn test_maven_default_package_test_class_logic() {
    let project = TestProject::new("maven_default_class", "maven", None);
    let stdout = project
        .task("java-test-class")
        .package("")
        .class("MyTest")
        .run();

    assert!(
        stdout.contains("-Dtest=MyTest"),
        "Should not include leading dot for test class in default package. Got: {}",
        stdout
    );
}

#[test]
fn test_maven_nested_test_method_logic() {
    let project = TestProject::new("maven_nested_method", "maven", None);
    let stdout = project
        .task("java-test-method")
        .package("example")
        .outer_class("NestedTest")
        .class("Inner")
        .method("testMethod")
        .run();

    assert!(
        stdout.contains("-Dtest=example.NestedTest$Inner#testMethod"),
        "Should correctly format nested test method for Maven. Got: {}",
        stdout
    );
}

#[test]
fn test_maven_nested_test_class_logic() {
    let project = TestProject::new("maven_nested_test_class", "maven", None);
    let stdout = project
        .task("java-test-class")
        .package("example")
        .outer_class("NestedTest")
        .class("Inner")
        .run();

    assert!(
        stdout.contains("-Dtest=example.NestedTest$Inner"),
        "Should correctly format nested test class for Maven. Got: {}",
        stdout
    );
}

// --- Gradle Tests ---

#[test]
fn test_gradle_single_module_command_logic() {
    let project = TestProject::new("gradle_single", "gradle", None);

    let stdout = project.task("java-main").run();

    assert!(
        stdout.contains("GRADLE_CALLED: :run"),
        "Should run as single module. Got: {}",
        stdout
    );
}

#[test]
fn test_gradle_multi_module_command_logic() {
    let project = TestProject::new("gradle_multi", "gradle", Some("module-a"));

    let stdout = project.task("java-main").run();

    assert!(
        stdout.contains("GRADLE_CALLED: :module-a:run"),
        "Should run with correct module path. Got: {}",
        stdout
    );
}

#[test]
fn test_gradle_nested_module_command_logic() {
    let project = TestProject::new("gradle_nested", "gradle", Some("nested/module-b"));

    let stdout = project.task("java-main").run();

    assert!(
        stdout.contains("GRADLE_CALLED: :nested:module-b:run"),
        "Should run with correct nested module path. Got: {}",
        stdout
    );
}

#[test]
fn test_gradle_multi_module_test_method_logic() {
    let project = TestProject::new("gradle_method", "gradle", Some("module-a"));

    let stdout = project
        .task("java-test-method")
        .method("shouldPersist")
        .run();

    assert!(
        stdout.contains("GRADLE_CALLED: :module-a:test --tests com.example.Main.shouldPersist"),
        "Should run submodule test with correct path. Got: {}",
        stdout
    );
}

#[test]
fn test_gradle_run_all_tests_logic() {
    let project = TestProject::new("gradle_all_tests", "gradle", Some("module-a"));

    let stdout = project.task("java-test-all").run();

    assert!(
        stdout.contains("GRADLE_CALLED: :module-a:test"),
        "Should run all tests in submodule. Got: {}",
        stdout
    );
}

#[test]
fn test_gradle_nested_class_test_method_logic() {
    let project = TestProject::new("gradle_nested_class", "gradle", Some("module-a"));

    let stdout = project
        .task("java-test-method")
        .class("Inner")
        .outer_class("Outer")
        .method("testMe")
        .run();

    assert!(
        stdout.contains("GRADLE_CALLED: :module-a:test --tests com.example.Outer$Inner.testMe"),
        "Should run nested class test method correctly for Gradle. Got: {}",
        stdout
    );
}

#[test]
fn test_gradle_test_class_logic() {
    let project = TestProject::new("gradle_test_class", "gradle", Some("module-a"));

    let stdout = project.task("java-test-class").run();

    assert!(
        stdout.contains("GRADLE_CALLED: :module-a:test --tests com.example.Main"),
        "Should run only the submodule test class for Gradle. Got: {}",
        stdout
    );
}

#[test]
fn test_gradle_single_level_package_logic() {
    let project = TestProject::new("gradle_single_package", "gradle", None);
    let stdout = project
        .task("java-main")
        .package("example")
        .class("Main")
        .run();

    assert!(
        stdout.contains("-PmainClass=example.Main"),
        "Should include the single-level package in Gradle. Got: {}",
        stdout
    );
}

#[test]
fn test_gradle_default_package_command_logic() {
    let project = TestProject::new("gradle_default_package", "gradle", None);
    let stdout = project.task("java-main").package("").class("Main").run();

    assert!(
        stdout.contains("-PmainClass=Main"),
        "Should not include leading dot for default package in Gradle. Got: {}",
        stdout
    );
}

#[test]
fn test_gradle_default_package_test_method_logic() {
    let project = TestProject::new("gradle_default_test", "gradle", None);
    let stdout = project
        .task("java-test-method")
        .package("")
        .class("MyTest")
        .method("testMethod")
        .run();

    assert!(
        stdout.contains("--tests MyTest.testMethod"),
        "Should not include leading dot for test method in default package (Gradle). Got: {}",
        stdout
    );
}

#[test]
fn test_gradle_default_package_test_class_logic() {
    let project = TestProject::new("gradle_default_class", "gradle", None);
    let stdout = project
        .task("java-test-class")
        .package("")
        .class("MyTest")
        .run();

    assert!(
        stdout.contains("--tests MyTest"),
        "Should not include leading dot for test class in default package (Gradle). Got: {}",
        stdout
    );
}

#[test]
fn test_gradle_nested_test_method_logic() {
    let project = TestProject::new("gradle_nested_test", "gradle", None);
    let stdout = project
        .task("java-test-method")
        .package("example")
        .outer_class("NestedTest")
        .class("Inner")
        .method("testMethod")
        .run();

    assert!(
        stdout.contains("--tests example.NestedTest$Inner.testMethod"),
        "Should correctly format nested test for Gradle. Got: {}",
        stdout
    );
}

#[test]
fn test_gradle_nested_test_class_logic() {
    let project = TestProject::new("gradle_nested_test_class", "gradle", None);
    let stdout = project
        .task("java-test-method")
        .package("example")
        .outer_class("NestedTest")
        .class("Inner")
        .run();

    assert!(
        stdout.contains("--tests example.NestedTest$Inner"),
        "Should correctly format nested test for Gradle. Got: {}",
        stdout
    );
}

// --- Generic Tests ---

#[test]
fn test_no_build_tool_command_logic() {
    let project = TestProject::new("no_build_tool", "none", None);
    project.mock_bin("javac", "#!/bin/sh\necho \"JAVAC_CALLED: $@\"");
    project.mock_bin("java", "#!/bin/sh\necho \"JAVA_CALLED: $@\"");

    let stdout = project.task("java-main").run();

    assert!(
        stdout.contains("JAVAC_CALLED: -d bin"),
        "Should compile with javac. Got: {}",
        stdout
    );
    assert!(
        stdout.contains(&project.zed_file.to_string_lossy().to_string()),
        "Should compile Main.java. Got: {}",
        stdout
    );
    assert!(
        stdout.contains("JAVA_CALLED: -cp bin com.example.Main"),
        "Should run with java. Got: {}",
        stdout
    );
}
