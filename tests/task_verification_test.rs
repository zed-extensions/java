use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

fn setup_mock_project(
    temp_dir: &Path,
    project_type: &str,
    is_multi: bool,
) -> (PathBuf, PathBuf) {
    let module_dir = if is_multi {
        temp_dir.join("module-a")
    } else {
        temp_dir.to_path_buf()
    };
    
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
    } else {
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

#[test]
fn test_maven_multi_module_command_logic() {
    use std::process::Command as StdCommand;

    let tasks_json = fs::read_to_string("languages/java/tasks.json").expect("Failed to read tasks.json");
    let tasks: Value = serde_json::from_str(&tasks_json).expect("Failed to parse tasks.json");
    let mut run_command = tasks[0]["command"].as_str().expect("Command is not a string").to_string();

    run_command = run_command.replace("${ZED_CUSTOM_java_outer_class_name:}", "${ZED_CUSTOM_java_outer_class_name:-}");

    let temp_dir = std::env::temp_dir().join("zed_java_test_maven_logic_integration");
    if temp_dir.exists() {
        fs::remove_dir_all(&temp_dir).unwrap();
    }
    fs::create_dir_all(&temp_dir).unwrap();
    
    let (zed_file, bin_dir) = setup_mock_project(&temp_dir, "maven", true);

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
fn test_maven_multi_module_test_method_logic() {
    use std::process::Command as StdCommand;

    let tasks_json = fs::read_to_string("languages/java/tasks.json").expect("Failed to read tasks.json");
    let tasks: Value = serde_json::from_str(&tasks_json).expect("Failed to parse tasks.json");
    let mut test_command = tasks[1]["command"].as_str().expect("Command is not a string").to_string();

    test_command = test_command.replace("${ZED_CUSTOM_java_outer_class_name:}", "${ZED_CUSTOM_java_outer_class_name:-}");

    let temp_dir = std::env::temp_dir().join("zed_java_test_maven_method_logic_integration");
    if temp_dir.exists() {
        fs::remove_dir_all(&temp_dir).unwrap();
    }
    fs::create_dir_all(&temp_dir).unwrap();
    
    let (zed_file, bin_dir) = setup_mock_project(&temp_dir, "maven", true);

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

    let tasks_json = fs::read_to_string("languages/java/tasks.json").expect("Failed to read tasks.json");
    let tasks: Value = serde_json::from_str(&tasks_json).expect("Failed to parse tasks.json");
    let mut run_command = tasks[0]["command"].as_str().expect("Command is not a string").to_string();

    run_command = run_command.replace("${ZED_CUSTOM_java_outer_class_name:}", "${ZED_CUSTOM_java_outer_class_name:-}");

    let temp_dir = std::env::temp_dir().join("zed_java_test_gradle_logic_integration");
    if temp_dir.exists() {
        fs::remove_dir_all(&temp_dir).unwrap();
    }
    fs::create_dir_all(&temp_dir).unwrap();
    
    let (zed_file, bin_dir) = setup_mock_project(&temp_dir, "gradle", true);

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
fn test_gradle_multi_module_test_method_logic() {
    use std::process::Command as StdCommand;

    let tasks_json = fs::read_to_string("languages/java/tasks.json").expect("Failed to read tasks.json");
    let tasks: Value = serde_json::from_str(&tasks_json).expect("Failed to parse tasks.json");
    let mut test_command = tasks[1]["command"].as_str().expect("Command is not a string").to_string();

    test_command = test_command.replace("${ZED_CUSTOM_java_outer_class_name:}", "${ZED_CUSTOM_java_outer_class_name:-}");

    let temp_dir = std::env::temp_dir().join("zed_java_test_gradle_method_logic_integration");
    if temp_dir.exists() {
        fs::remove_dir_all(&temp_dir).unwrap();
    }
    fs::create_dir_all(&temp_dir).unwrap();
    
    let (zed_file, bin_dir) = setup_mock_project(&temp_dir, "gradle", true);

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
