use crate::command::TaskCommand;
use std::env;
use std::path::{Path, PathBuf};

pub mod gradle;
pub mod maven;
pub mod vanilla;

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
    let cwd = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
    if cwd.join("pom.xml").exists() {
        return (Box::new(maven::Maven::new(cwd.clone())), cwd);
    }
    if cwd.join("build.gradle").exists()
        || cwd.join("build.gradle.kts").exists()
        || cwd.join("settings.gradle").exists()
        || cwd.join("settings.gradle.kts").exists()
    {
        return (Box::new(gradle::Gradle::new(cwd.clone())), cwd);
    }
    (Box::new(vanilla::Vanilla::new(cwd.clone())), cwd)
}

pub fn get_workspace_root() -> (Box<dyn BuildTool>, PathBuf) {
    let cwd = env::current_dir().unwrap_or_default();
    detect_build_tool(&cwd)
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

    let abs_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());

    let mut current_opt = current_abs;
    while let Some(current) = current_opt {
        let current_canonical = current.canonicalize().unwrap_or_else(|_| current.clone());
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

pub fn which_wrapper(root: &Path, tool_name: &str) -> String {
    let wrapper_name = if tool_name == "mvn" {
        if cfg!(windows) {
            "mvnw.cmd"
        } else {
            "./mvnw"
        }
    } else {
        if cfg!(windows) {
            "gradlew.bat"
        } else {
            "./gradlew"
        }
    };

    if root.join(wrapper_name.trim_start_matches("./")).exists() {
        wrapper_name.to_string()
    } else {
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
