use crate::command::TaskCommand;
use std::env;
use std::path::{Path, PathBuf};

pub mod gradle;
pub mod maven;
pub mod vanilla;

fn canonicalize_clean(path: &Path) -> PathBuf {
    let c = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let s = c.to_string_lossy();
    if let Some(stripped) = s.strip_prefix(r"\\?\") {
        PathBuf::from(stripped)
    } else {
        c
    }
}

pub fn full_class_name(package: &str, class: &str, outer: Option<&str>) -> String {
    let full_class = match outer {
        Some(o) => format!("{}${}", o, class),
        None => class.to_string(),
    };
    if package.is_empty() {
        full_class
    } else {
        format!("{}.{}", package, full_class)
    }
}

pub fn task(
    command: String,
    args: Vec<String>,
    cwd: String,
    env: Vec<(String, String)>,
) -> TaskCommand {
    TaskCommand {
        command,
        args,
        cwd,
        env,
        then: vec![],
    }
}

pub trait BuildTool {
    fn run_class(&self, file: &str, package: &str, class: &str, outer: Option<&str>)
        -> TaskCommand;
    fn run_test_method(
        &self,
        file: &str,
        package: &str,
        class: &str,
        outer: Option<&str>,
        method: &str,
    ) -> TaskCommand;
    fn run_test_class(
        &self,
        file: &str,
        package: &str,
        class: &str,
        outer: Option<&str>,
    ) -> TaskCommand;
    fn run_all_tests(&self, file: &str) -> TaskCommand;
}

pub fn detect_build_tool(cwd: &Path) -> (Box<dyn BuildTool>, PathBuf) {
    let cwd = canonicalize_clean(cwd);

    // Walk up from cwd to find the highest directory with a build marker file.
    // This determines the project root, ensuring gradlew/mvnw are found
    // regardless of where in the project tree the user opened the file.
    let root = walk_up_to_highest_marker(&cwd).unwrap_or(cwd);

    if root.join("pom.xml").exists() {
        (Box::new(maven::Maven::new(root.clone())), root)
    } else if root.join("build.gradle").exists()
        || root.join("build.gradle.kts").exists()
        || root.join("settings.gradle").exists()
        || root.join("settings.gradle.kts").exists()
    {
        (Box::new(gradle::Gradle::new(root.clone())), root)
    } else {
        (Box::new(vanilla::Vanilla::new(root.clone())), root)
    }
}

/// Walk up from `start` to the filesystem root, returning the *highest*
/// directory that contains a build marker file (pom.xml, build.gradle,
/// build.gradle.kts, settings.gradle, settings.gradle.kts).
fn walk_up_to_highest_marker(start: &Path) -> Option<PathBuf> {
    let mut current = Some(start);
    let mut highest = None;
    while let Some(dir) = current {
        if dir.join("pom.xml").exists()
            || dir.join("build.gradle").exists()
            || dir.join("build.gradle.kts").exists()
            || dir.join("settings.gradle").exists()
            || dir.join("settings.gradle.kts").exists()
        {
            highest = Some(dir.to_path_buf());
        }
        current = dir.parent();
    }
    highest
}

pub fn get_workspace_root(file: Option<&str>) -> (Box<dyn BuildTool>, PathBuf) {
    let start = file
        .and_then(|f| Path::new(f).parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| env::current_dir().unwrap_or_default());
    detect_build_tool(&start)
}

pub fn find_closest_module(file_path: &str, root: &Path, marker_files: &[&str]) -> Option<PathBuf> {
    let file_path = Path::new(file_path);
    let current_abs = if file_path.is_absolute() {
        if file_path.is_file() {
            file_path.parent().map(|p| p.to_path_buf())
        } else {
            Some(file_path.to_path_buf())
        }
    } else {
        let abs = root.join(file_path);
        if abs.is_file() {
            abs.parent().map(|p| p.to_path_buf())
        } else {
            Some(abs)
        }
    };

    let abs_root = canonicalize_clean(root);

    let mut current_opt = current_abs;
    while let Some(current) = current_opt {
        let current_canonical = canonicalize_clean(&current);
        if !current_canonical.starts_with(&abs_root) {
            break;
        }

        for marker in marker_files {
            if current.join(marker).exists() {
                let rel = current_canonical
                    .strip_prefix(&abs_root)
                    .ok()
                    .map(|p| p.to_path_buf());
                if let Some(ref p) = rel {
                    if p.as_os_str().is_empty() {
                        return None; // Root module
                    }
                }
                return rel;
            }
        }

        if current_canonical == abs_root {
            break;
        }

        current_opt = current.parent().map(|p| p.to_path_buf());
    }
    None
}

#[cfg(windows)]
pub fn which_wrapper(root: &Path, tool_name: &str) -> String {
    let wrapper_path = root.join("gradlew.bat");
    if wrapper_path.exists() {
        eprintln!("Detected {} project. Using gradlew.bat wrapper.", tool_name);
        return wrapper_path.to_string_lossy().to_string();
    }
    let wrapper_path = root.join("mvnw.cmd");
    if wrapper_path.exists() {
        eprintln!("Detected {} project. Using mvnw.cmd wrapper.", tool_name);
        return wrapper_path.to_string_lossy().to_string();
    }
    eprintln!(
        "Detected {} project. No wrapper found, using {}.",
        tool_name, tool_name
    );
    tool_name.to_string()
}

#[cfg(not(windows))]
pub fn which_wrapper(root: &Path, tool_name: &str) -> String {
    let wrapper_path = root.join("gradlew");
    if wrapper_path.exists() {
        eprintln!("Detected {} project. Using ./gradlew wrapper.", tool_name);
        "./gradlew".to_string()
    } else if root.join("mvnw").exists() {
        eprintln!("Detected {} project. Using ./mvnw wrapper.", tool_name);
        "./mvnw".to_string()
    } else {
        eprintln!(
            "Detected {} project. No wrapper found, using {}.",
            tool_name, tool_name
        );
        tool_name.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};
    use tempfile::tempdir;

    #[test]
    fn test_detect_maven() {
        let dir = tempdir().unwrap();
        File::create(dir.path().join("pom.xml")).unwrap();

        let (_tool, root) = detect_build_tool(dir.path());
        // Since we return Box<dyn BuildTool>, we can't easily assert type,
        // but we can check the root.
        assert_eq!(root, dir.path().canonicalize().unwrap());
    }

    #[test]
    fn test_detect_build_tool_walks_up_to_highest_marker() {
        // Multi-module project: root has settings.gradle, submodule has build.gradle.
        // detect_build_tool should return the root (highest), not the submodule.
        let root_dir = tempdir().unwrap();
        let root_path = root_dir.path().canonicalize().unwrap();
        File::create(root_path.join("settings.gradle")).unwrap();

        let sub_dir = root_path.join("module-a");
        fs::create_dir(&sub_dir).unwrap();
        File::create(sub_dir.join("build.gradle")).unwrap();

        let deep_path = sub_dir.join("src/main/java/com/example/App.java");
        fs::create_dir_all(deep_path.parent().unwrap()).unwrap();
        File::create(&deep_path).unwrap();

        // Call detect_build_tool with a deep file dir — should walk up to root
        let (_tool, root) = detect_build_tool(deep_path.parent().unwrap());
        assert_eq!(root, root_path);

        // Call from submodule dir — should still walk up to root
        let (_tool, root) = detect_build_tool(&sub_dir);
        assert_eq!(root, root_path);
    }

    #[test]
    fn test_detect_build_tool_walks_up_from_deep_directory() {
        // Project with pom.xml at root, file deep in tree.
        // detect_build_tool should walk up and find the root.
        let dir = tempdir().unwrap();
        let dir_path = dir.path().canonicalize().unwrap();
        File::create(dir_path.join("pom.xml")).unwrap();

        let deep_dir = dir_path.join("src/main/java/com/example");
        fs::create_dir_all(&deep_dir).unwrap();

        let (_tool, root) = detect_build_tool(&deep_dir);
        assert_eq!(root, dir_path);
    }

    #[test]
    fn test_detect_build_tool_no_marker_returns_vanilla_and_cwd() {
        let dir = tempdir().unwrap();
        let dir_path = dir.path().canonicalize().unwrap();
        let (_tool, root) = detect_build_tool(&dir_path);
        assert_eq!(root, dir_path);
    }

    #[test]
    fn test_detect_build_tool_single_module() {
        // Simple single-module project: just build.gradle, no settings file.
        let dir = tempdir().unwrap();
        let dir_path = dir.path().canonicalize().unwrap();
        File::create(dir_path.join("build.gradle")).unwrap();

        let file = dir_path.join("src/main/java/App.java");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        File::create(&file).unwrap();

        let (_tool, root) = detect_build_tool(file.parent().unwrap());
        assert_eq!(root, dir_path);
    }

    #[test]
    fn test_find_closest_module_maven() {
        let root_dir = tempdir().unwrap();
        let root_path = root_dir.path().canonicalize().unwrap();
        File::create(root_path.join("pom.xml")).unwrap();

        let sub_dir = root_path.join("module-a");
        fs::create_dir(&sub_dir).unwrap();
        File::create(sub_dir.join("pom.xml")).unwrap();

        let file_path = sub_dir.join("src/main/java/App.java");
        fs::create_dir_all(file_path.parent().unwrap()).unwrap();
        File::create(&file_path).unwrap();

        let module = find_closest_module(file_path.to_str().unwrap(), &root_path, &["pom.xml"]);
        assert_eq!(module, Some(PathBuf::from("module-a")));
    }

    #[test]
    fn test_find_closest_module_gradle() {
        let root_dir = tempdir().unwrap();
        let root_path = root_dir.path().canonicalize().unwrap();
        File::create(root_path.join("settings.gradle")).unwrap();

        let sub_dir = root_path.join("module-b");
        fs::create_dir(&sub_dir).unwrap();
        File::create(sub_dir.join("build.gradle")).unwrap();

        let file_path = sub_dir.join("src/main/java/App.java");
        fs::create_dir_all(file_path.parent().unwrap()).unwrap();
        File::create(&file_path).unwrap();

        let module =
            find_closest_module(file_path.to_str().unwrap(), &root_path, &["build.gradle"]);
        assert_eq!(module, Some(PathBuf::from("module-b")));
    }
}
