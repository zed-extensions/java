use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

fn setup_mock_project(
    temp_dir: &Path,
    project_type: &str,
    module_path: Option<&str>,
) -> (PathBuf, PathBuf) {
    let module_dir = if let Some(path) = module_path {
        let d = temp_dir.join(path);
        fs::create_dir_all(&d).unwrap();
        d
    } else {
        temp_dir.to_path_buf()
    };
    
    let is_multi = module_path.is_some();
    let package_dir = if project_type == "maven" && is_multi {
         module_dir.join("src/test/java/com/example")
    } else {
         module_dir.join("src/main/java/com/example")
    };
    
    fs::create_dir_all(&package_dir).unwrap();

    let bin_dir = temp_dir.join("bin");
    fs::create_dir_all(&bin_dir).unwrap();

    if project_type == "maven" {
        fs::File::create(temp_dir.join("pom.xml")).unwrap();
        if is_multi {
            fs::File::create(module_dir.join("pom.xml")).unwrap();
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
        if is_multi {
            fs::File::create(module_dir.join("build.gradle")).unwrap();
        }
        let gradle_mock = bin_dir.join("gradle");
        fs::write(&gradle_mock, "#!/bin/sh\necho \"GRADLE_CALLED: $@\"").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&gradle_mock, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    let zed_file = package_dir.join("Main.java");
    fs::File::create(&zed_file).unwrap();

    (zed_file, bin_dir)
}

fn get_task_command_by_tag(tag: &str) -> String {
    let tasks_json = fs::read_to_string("languages/java/tasks.json").expect("Failed to read tasks.json");
    let tasks: Value = serde_json::from_str(&tasks_json).expect("Failed to parse tasks.json");
    let tasks_array = tasks.as_array().expect("tasks.json is not an array");

    for task in tasks_array {
        if let Some(tags) = task["tags"].as_array() {
            if tags.iter().any(|t| t.as_str() == Some(tag)) {
                return task["command"].as_str().expect("Command is not a string").to_string();
            }
        }
    }
    panic!("Task with tag '{}' not found", tag);
}

#[test]
fn test_maven_multi_module_command_logic() {
    use std::process::Command as StdCommand;

    let mut run_command = get_task_command_by_tag("java-main");

    run_command = run_command.replace("${ZED_CUSTOM_java_outer_class_name:}", "${ZED_CUSTOM_java_outer_class_name:-}");

    let temp_dir = std::env::temp_dir().join("zed_java_test_maven_logic_integration");
    if temp_dir.exists() {
        fs::remove_dir_all(&temp_dir).unwrap();
    }
    fs::create_dir_all(&temp_dir).unwrap();
    
    let (zed_file, bin_dir) = setup_mock_project(&temp_dir, "maven", Some("module-a"));

    let old_path = std::env::var("PATH").unwrap_or_default();
    let new_path = format!("{}:{}", bin_dir.to_string_lossy(), old_path);

    let output = StdCommand::new("sh")
        .arg("-c")
        .arg(&run_command)
        .env("ZED_FILE", zed_file.to_string_lossy().to_string())
        .env("PWD", temp_dir.to_string_lossy().to_string())
        .env("ZED_CUSTOM_java_package_name", "com.example")
        .env("ZED_CUSTOM_java_class_name", "Main")
        .env("PATH", new_path)
        .current_dir(&temp_dir)
        .output()
        .expect("Failed to execute shell command");

    let stdout = String::from_utf8_lossy(&output.stdout);
    
    assert!(stdout.contains("MVN_CALLED: clean test-compile -pl module-a -am"), "Should build submodule with dependencies. Got: {}", stdout);
    assert!(stdout.contains("MVN_CALLED: exec:java -pl module-a"), "Should run only the submodule. Got: {}", stdout);
    assert!(stdout.contains("-Dexec.classpathScope=test"), "Should use test classpath scope. Got: {}", stdout);

    fs::remove_dir_all(&temp_dir).unwrap();
}

#[test]
fn test_maven_single_module_command_logic() {
    use std::process::Command as StdCommand;

    let mut run_command = get_task_command_by_tag("java-main");

    run_command = run_command.replace("${ZED_CUSTOM_java_outer_class_name:}", "${ZED_CUSTOM_java_outer_class_name:-}");

    let temp_dir = std::env::temp_dir().join("zed_java_test_maven_single_logic_integration");
    if temp_dir.exists() {
        fs::remove_dir_all(&temp_dir).unwrap();
    }
    fs::create_dir_all(&temp_dir).unwrap();
    
    let (zed_file, bin_dir) = setup_mock_project(&temp_dir, "maven", None);

    let old_path = std::env::var("PATH").unwrap_or_default();
    let new_path = format!("{}:{}", bin_dir.to_string_lossy(), old_path);

    let output = StdCommand::new("sh")
        .arg("-c")
        .arg(&run_command)
        .env("ZED_FILE", zed_file.to_string_lossy().to_string())
        .env("PWD", temp_dir.to_string_lossy().to_string())
        .env("ZED_CUSTOM_java_package_name", "com.example")
        .env("ZED_CUSTOM_java_class_name", "Main")
        .env("PATH", new_path)
        .current_dir(&temp_dir)
        .output()
        .expect("Failed to execute shell command");

    let stdout = String::from_utf8_lossy(&output.stdout);
    
    assert!(stdout.contains("MVN_CALLED: clean test-compile exec:java -Dexec.mainClass=com.example.Main"), "Should run as single module. Got: {}", stdout);
    assert!(stdout.contains("-Dexec.classpathScope=test"), "Should use test classpath scope. Got: {}", stdout);

    fs::remove_dir_all(&temp_dir).unwrap();
}

#[test]
fn test_maven_nested_module_command_logic() {
    use std::process::Command as StdCommand;

    let mut run_command = get_task_command_by_tag("java-main");

    run_command = run_command.replace("${ZED_CUSTOM_java_outer_class_name:}", "${ZED_CUSTOM_java_outer_class_name:-}");

    let temp_dir = std::env::temp_dir().join("zed_java_test_maven_nested_logic_integration");
    if temp_dir.exists() {
        fs::remove_dir_all(&temp_dir).unwrap();
    }
    fs::create_dir_all(&temp_dir).unwrap();
    
    let (zed_file, bin_dir) = setup_mock_project(&temp_dir, "maven", Some("nested/module-b"));

    let old_path = std::env::var("PATH").unwrap_or_default();
    let new_path = format!("{}:{}", bin_dir.to_string_lossy(), old_path);

    let output = StdCommand::new("sh")
        .arg("-c")
        .arg(&run_command)
        .env("ZED_FILE", zed_file.to_string_lossy().to_string())
        .env("PWD", temp_dir.to_string_lossy().to_string())
        .env("ZED_CUSTOM_java_package_name", "com.example")
        .env("ZED_CUSTOM_java_class_name", "Main")
        .env("PATH", new_path)
        .current_dir(&temp_dir)
        .output()
        .expect("Failed to execute shell command");

    let stdout = String::from_utf8_lossy(&output.stdout);
    
    assert!(stdout.contains("MVN_CALLED: clean test-compile -pl nested/module-b -am"), "Should build nested submodule with dependencies. Got: {}", stdout);
    assert!(stdout.contains("MVN_CALLED: exec:java -pl nested/module-b"), "Should run only the nested submodule. Got: {}", stdout);

    fs::remove_dir_all(&temp_dir).unwrap();
}

#[test]
fn test_maven_multi_module_test_method_logic() {
    use std::process::Command as StdCommand;

    let mut test_command = get_task_command_by_tag("java-test-method");

    test_command = test_command.replace("${ZED_CUSTOM_java_outer_class_name:}", "${ZED_CUSTOM_java_outer_class_name:-}");

    let temp_dir = std::env::temp_dir().join("zed_java_test_maven_method_logic_integration");
    if temp_dir.exists() {
        fs::remove_dir_all(&temp_dir).unwrap();
    }
    fs::create_dir_all(&temp_dir).unwrap();
    
    let (zed_file, bin_dir) = setup_mock_project(&temp_dir, "maven", Some("module-a"));

    let old_path = std::env::var("PATH").unwrap_or_default();
    let new_path = format!("{}:{}", bin_dir.to_string_lossy(), old_path);

    let output = StdCommand::new("sh")
        .arg("-c")
        .arg(&test_command)
        .env("ZED_FILE", zed_file.to_string_lossy().to_string())
        .env("PWD", temp_dir.to_string_lossy().to_string())
        .env("ZED_CUSTOM_java_package_name", "com.example")
        .env("ZED_CUSTOM_java_class_name", "Main")
        .env("ZED_CUSTOM_java_method_name", "shouldPersist")
        .env("PATH", new_path)
        .current_dir(&temp_dir)
        .output()
        .expect("Failed to execute shell command");

    let stdout = String::from_utf8_lossy(&output.stdout);
    
    assert!(stdout.contains("MVN_CALLED: clean test-compile -pl module-a -am"), "Should build submodule with dependencies. Got: {}", stdout);
    assert!(stdout.contains("MVN_CALLED: test -pl module-a -Dtest=com.example.Main#shouldPersist"), "Should run only the submodule test. Got: {}", stdout);

    fs::remove_dir_all(&temp_dir).unwrap();
}

#[test]
fn test_gradle_multi_module_command_logic() {
    use std::process::Command as StdCommand;

    let mut run_command = get_task_command_by_tag("java-main");

    run_command = run_command.replace("${ZED_CUSTOM_java_outer_class_name:}", "${ZED_CUSTOM_java_outer_class_name:-}");

    let temp_dir = std::env::temp_dir().join("zed_java_test_gradle_logic_integration");
    if temp_dir.exists() {
        fs::remove_dir_all(&temp_dir).unwrap();
    }
    fs::create_dir_all(&temp_dir).unwrap();
    
    let (zed_file, bin_dir) = setup_mock_project(&temp_dir, "gradle", Some("module-a"));

    let old_path = std::env::var("PATH").unwrap_or_default();
    let new_path = format!("{}:{}", bin_dir.to_string_lossy(), old_path);

    let output = StdCommand::new("sh")
        .arg("-c")
        .arg(&run_command)
        .env("ZED_FILE", zed_file.to_string_lossy().to_string())
        .env("PWD", temp_dir.to_string_lossy().to_string())
        .env("ZED_CUSTOM_java_package_name", "com.example")
        .env("ZED_CUSTOM_java_class_name", "Main")
        .env("PATH", new_path)
        .current_dir(&temp_dir)
        .output()
        .expect("Failed to execute shell command");

    let stdout = String::from_utf8_lossy(&output.stdout);
    
    assert!(stdout.contains("GRADLE_CALLED: :module-a:run"), "Should run with correct module path. Got: {}", stdout);

    fs::remove_dir_all(&temp_dir).unwrap();
}

#[test]
fn test_gradle_single_module_command_logic() {
    use std::process::Command as StdCommand;

    let mut run_command = get_task_command_by_tag("java-main");

    run_command = run_command.replace("${ZED_CUSTOM_java_outer_class_name:}", "${ZED_CUSTOM_java_outer_class_name:-}");

    let temp_dir = std::env::temp_dir().join("zed_java_test_gradle_single_logic_integration");
    if temp_dir.exists() {
        fs::remove_dir_all(&temp_dir).unwrap();
    }
    fs::create_dir_all(&temp_dir).unwrap();
    
    let (zed_file, bin_dir) = setup_mock_project(&temp_dir, "gradle", None);

    let old_path = std::env::var("PATH").unwrap_or_default();
    let new_path = format!("{}:{}", bin_dir.to_string_lossy(), old_path);

    let output = StdCommand::new("sh")
        .arg("-c")
        .arg(&run_command)
        .env("ZED_FILE", zed_file.to_string_lossy().to_string())
        .env("PWD", temp_dir.to_string_lossy().to_string())
        .env("ZED_CUSTOM_java_package_name", "com.example")
        .env("ZED_CUSTOM_java_class_name", "Main")
        .env("PATH", new_path)
        .current_dir(&temp_dir)
        .output()
        .expect("Failed to execute shell command");

    let stdout = String::from_utf8_lossy(&output.stdout);
    
    assert!(stdout.contains("GRADLE_CALLED: :run"), "Should run as single module. Got: {}", stdout);

    fs::remove_dir_all(&temp_dir).unwrap();
}

#[test]
fn test_gradle_nested_module_command_logic() {
    use std::process::Command as StdCommand;

    let mut run_command = get_task_command_by_tag("java-main");

    run_command = run_command.replace("${ZED_CUSTOM_java_outer_class_name:}", "${ZED_CUSTOM_java_outer_class_name:-}");

    let temp_dir = std::env::temp_dir().join("zed_java_test_gradle_nested_logic_integration");
    if temp_dir.exists() {
        fs::remove_dir_all(&temp_dir).unwrap();
    }
    fs::create_dir_all(&temp_dir).unwrap();
    
    let (zed_file, bin_dir) = setup_mock_project(&temp_dir, "gradle", Some("nested/module-b"));

    let old_path = std::env::var("PATH").unwrap_or_default();
    let new_path = format!("{}:{}", bin_dir.to_string_lossy(), old_path);

    let output = StdCommand::new("sh")
        .arg("-c")
        .arg(&run_command)
        .env("ZED_FILE", zed_file.to_string_lossy().to_string())
        .env("PWD", temp_dir.to_string_lossy().to_string())
        .env("ZED_CUSTOM_java_package_name", "com.example")
        .env("ZED_CUSTOM_java_class_name", "Main")
        .env("PATH", new_path)
        .current_dir(&temp_dir)
        .output()
        .expect("Failed to execute shell command");

    let stdout = String::from_utf8_lossy(&output.stdout);
    
    assert!(stdout.contains("GRADLE_CALLED: :nested:module-b:run"), "Should run with correct nested module path. Got: {}", stdout);

    fs::remove_dir_all(&temp_dir).unwrap();
}

#[test]
fn test_gradle_multi_module_test_method_logic() {
    use std::process::Command as StdCommand;

    let mut test_command = get_task_command_by_tag("java-test-method");

    test_command = test_command.replace("${ZED_CUSTOM_java_outer_class_name:}", "${ZED_CUSTOM_java_outer_class_name:-}");

    let temp_dir = std::env::temp_dir().join("zed_java_test_gradle_method_logic_integration");
    if temp_dir.exists() {
        fs::remove_dir_all(&temp_dir).unwrap();
    }
    fs::create_dir_all(&temp_dir).unwrap();
    
    let (zed_file, bin_dir) = setup_mock_project(&temp_dir, "gradle", Some("module-a"));

    let old_path = std::env::var("PATH").unwrap_or_default();
    let new_path = format!("{}:{}", bin_dir.to_string_lossy(), old_path);

    let output = StdCommand::new("sh")
        .arg("-c")
        .arg(&test_command)
        .env("ZED_FILE", zed_file.to_string_lossy().to_string())
        .env("PWD", temp_dir.to_string_lossy().to_string())
        .env("ZED_CUSTOM_java_package_name", "com.example")
        .env("ZED_CUSTOM_java_class_name", "Main")
        .env("ZED_CUSTOM_java_method_name", "shouldPersist")
        .env("PATH", new_path)
        .current_dir(&temp_dir)
        .output()
        .expect("Failed to execute shell command");

    let stdout = String::from_utf8_lossy(&output.stdout);
    
    assert!(stdout.contains("GRADLE_CALLED: :module-a:test --tests com.example.Main.shouldPersist"), "Should run submodule test with correct path. Got: {}", stdout);

    fs::remove_dir_all(&temp_dir).unwrap();
}

#[test]
fn test_no_build_tool_command_logic() {
    use std::process::Command as StdCommand;

    let mut run_command = get_task_command_by_tag("java-main");

    run_command = run_command.replace("${ZED_CUSTOM_java_outer_class_name:}", "${ZED_CUSTOM_java_outer_class_name:-}");

    let temp_dir = std::env::temp_dir().join("zed_java_test_no_build_tool_logic_integration");
    if temp_dir.exists() {
        fs::remove_dir_all(&temp_dir).unwrap();
    }
    fs::create_dir_all(&temp_dir).unwrap();
    
    let (zed_file, bin_dir) = setup_mock_project(&temp_dir, "none", None);

    let old_path = std::env::var("PATH").unwrap_or_default();
    let new_path = format!("{}:{}", bin_dir.to_string_lossy(), old_path);

    // Mock javac and java
    let javac_mock = bin_dir.join("javac");
    fs::write(&javac_mock, "#!/bin/sh\necho \"JAVAC_CALLED: $@\"").unwrap();
    let java_mock = bin_dir.join("java");
    fs::write(&java_mock, "#!/bin/sh\necho \"JAVA_CALLED: $@\"").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&javac_mock, fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(&java_mock, fs::Permissions::from_mode(0o755)).unwrap();
    }

    let output = StdCommand::new("sh")
        .arg("-c")
        .arg(&run_command)
        .env("ZED_FILE", zed_file.to_string_lossy().to_string())
        .env("PWD", temp_dir.to_string_lossy().to_string())
        .env("ZED_CUSTOM_java_package_name", "com.example")
        .env("ZED_CUSTOM_java_class_name", "Main")
        .env("PATH", new_path)
        .current_dir(&temp_dir)
        .output()
        .expect("Failed to execute shell command");

    let stdout = String::from_utf8_lossy(&output.stdout);
    
    assert!(stdout.contains("JAVAC_CALLED: -d bin ./src/main/java/com/example/Main.java"), "Should compile with javac. Got: {}", stdout);
    assert!(stdout.contains("JAVA_CALLED: -cp bin com.example.Main"), "Should run with java. Got: {}", stdout);

    fs::remove_dir_all(&temp_dir).unwrap();
}

#[test]
fn test_maven_nested_class_test_method_logic() {
    use std::process::Command as StdCommand;

    let mut test_command = get_task_command_by_tag("java-test-method");

    test_command = test_command.replace("${ZED_CUSTOM_java_outer_class_name:}", "${ZED_CUSTOM_java_outer_class_name:-}");

    let temp_dir = std::env::temp_dir().join("zed_java_test_maven_nested_class_logic_integration");
    if temp_dir.exists() {
        fs::remove_dir_all(&temp_dir).unwrap();
    }
    fs::create_dir_all(&temp_dir).unwrap();
    
    let (zed_file, bin_dir) = setup_mock_project(&temp_dir, "maven", Some("module-a"));

    let old_path = std::env::var("PATH").unwrap_or_default();
    let new_path = format!("{}:{}", bin_dir.to_string_lossy(), old_path);

    let output = StdCommand::new("sh")
        .arg("-c")
        .arg(&test_command)
        .env("ZED_FILE", zed_file.to_string_lossy().to_string())
        .env("PWD", temp_dir.to_string_lossy().to_string())
        .env("ZED_CUSTOM_java_package_name", "com.example")
        .env("ZED_CUSTOM_java_class_name", "Inner")
        .env("ZED_CUSTOM_java_outer_class_name", "Outer")
        .env("ZED_CUSTOM_java_method_name", "testMe")
        .env("PATH", new_path)
        .current_dir(&temp_dir)
        .output()
        .expect("Failed to execute shell command");

    let stdout = String::from_utf8_lossy(&output.stdout);
    
    assert!(stdout.contains("MVN_CALLED: test -pl module-a -Dtest=com.example.Outer$Inner#testMe"), "Should run nested class test method correctly. Got: {}", stdout);

    fs::remove_dir_all(&temp_dir).unwrap();
}

#[test]
fn test_gradle_run_all_tests_logic() {
    use std::process::Command as StdCommand;

    let run_tests_command = get_task_command_by_tag("java-test-all");

    let temp_dir = std::env::temp_dir().join("zed_java_test_gradle_run_tests_integration");
    if temp_dir.exists() {
        fs::remove_dir_all(&temp_dir).unwrap();
    }
    fs::create_dir_all(&temp_dir).unwrap();
    
    let (zed_file, bin_dir) = setup_mock_project(&temp_dir, "gradle", Some("module-a"));

    let old_path = std::env::var("PATH").unwrap_or_default();
    let new_path = format!("{}:{}", bin_dir.to_string_lossy(), old_path);

    let output = StdCommand::new("sh")
        .arg("-c")
        .arg(&run_tests_command)
        .env("ZED_FILE", zed_file.to_string_lossy().to_string())
        .env("PWD", temp_dir.to_string_lossy().to_string())
        .env("PATH", new_path)
        .current_dir(&temp_dir)
        .output()
        .expect("Failed to execute shell command");

    let stdout = String::from_utf8_lossy(&output.stdout);
    
    assert!(stdout.contains("GRADLE_CALLED: :module-a:test"), "Should run all tests in submodule. Got: {}", stdout);

    fs::remove_dir_all(&temp_dir).unwrap();
}
