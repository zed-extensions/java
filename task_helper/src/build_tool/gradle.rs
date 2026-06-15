use crate::build_tool::{find_closest_module, which_wrapper, BuildTool};
use crate::command::TaskCommand;
use crate::is_debug;
use std::path::PathBuf;

pub struct Gradle {
    root: PathBuf,
}

impl Gradle {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn find_module(&self, file: &str) -> Option<PathBuf> {
        find_closest_module(file, &self.root, &["build.gradle", "build.gradle.kts"])
    }
}

impl BuildTool for Gradle {
    fn run_class(
        &self,
        file: &str,
        package: &str,
        class: &str,
        outer: Option<&str>,
    ) -> TaskCommand {
        let command = which_wrapper(&self.root, "gradle");
        let module = self.find_module(file);
        let full_class = match outer {
            Some(o) => format!("{}${}", o, class),
            None => class.to_string(),
        };
        let full_name = if package.is_empty() {
            full_class
        } else {
            format!("{}.{}", package, full_class)
        };

        let task = if let Some(m) = module {
            format!(":{}:run", m.to_string_lossy().replace("/", ":"))
        } else {
            ":run".to_string()
        };

        let mut args = vec![task, format!("-PmainClass={}", full_name)];
        if is_debug() {
            args.push("--debug-jvm".to_string());
        }

        TaskCommand {
            command,
            args,
            cwd: self.root.to_string_lossy().to_string(),
        }
    }

    fn run_test_method(
        &self,
        _file: &str,
        package: &str,
        class: &str,
        outer: Option<&str>,
        method: &str,
    ) -> TaskCommand {
        let command = which_wrapper(&self.root, "gradle");
        let module = self.find_module(_file);
        let full_class = match outer {
            Some(o) => format!("{}${}", o, class),
            None => class.to_string(),
        };
        let test_filter = if package.is_empty() {
            format!("{}.{}", full_class, method)
        } else {
            format!("{}.{}.{}", package, full_class, method)
        };

        let task = if let Some(m) = module {
            format!(":{}:test", m.to_string_lossy().replace("/", ":"))
        } else {
            ":test".to_string()
        };

        let mut args = vec![task, "--tests".to_string(), test_filter];
        if is_debug() {
            args.push("--debug-jvm".to_string());
        }

        TaskCommand {
            command,
            args,
            cwd: self.root.to_string_lossy().to_string(),
        }
    }

    fn run_test_class(
        &self,
        _file: &str,
        package: &str,
        class: &str,
        outer: Option<&str>,
    ) -> TaskCommand {
        let command = which_wrapper(&self.root, "gradle");
        let module = self.find_module(_file);
        let full_class = match outer {
            Some(o) => format!("{}${}", o, class),
            None => class.to_string(),
        };
        let test_filter = if package.is_empty() {
            full_class
        } else {
            format!("{}.{}", package, full_class)
        };

        let task = if let Some(m) = module {
            format!(":{}:test", m.to_string_lossy().replace("/", ":"))
        } else {
            ":test".to_string()
        };

        let mut args = vec![task, "--tests".to_string(), test_filter];
        if is_debug() {
            args.push("--debug-jvm".to_string());
        }

        TaskCommand {
            command,
            args,
            cwd: self.root.to_string_lossy().to_string(),
        }
    }

    fn run_all_tests(&self, _file: &str) -> TaskCommand {
        let command = which_wrapper(&self.root, "gradle");
        let module = self.find_module(_file);

        let task = if let Some(m) = module {
            format!(":{}:test", m.to_string_lossy().replace("/", ":"))
        } else {
            ":test".to_string()
        };

        let mut args = vec![task];
        if is_debug() {
            args.push("--debug-jvm".to_string());
        }

        TaskCommand {
            command,
            args,
            cwd: self.root.to_string_lossy().to_string(),
        }
    }
}
