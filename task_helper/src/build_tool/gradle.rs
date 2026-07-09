use crate::build_tool::{find_closest_module, full_class_name, task, which_wrapper, BuildTool};
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

    fn cwd(&self) -> String {
        self.root.to_string_lossy().to_string()
    }

    fn command(&self) -> String {
        which_wrapper(&self.root, "gradle")
    }

    fn gradle_task(&self, module: &Option<PathBuf>, task_name: &str) -> String {
        match module {
            Some(m) => {
                let gradle_path = m.to_string_lossy().replace(['/', '\\'], ":");
                format!(":{gradle_path}:{task_name}")
            }
            None => format!(":{task_name}"),
        }
    }

    fn debug_args() -> Vec<String> {
        if is_debug() {
            vec!["--debug-jvm".to_string()]
        } else {
            vec![]
        }
    }

    fn test_filter(
        package: &str,
        class: &str,
        outer: Option<&str>,
        method: Option<&str>,
    ) -> String {
        let full = full_class_name(package, class, outer);
        match method {
            Some(m) => format!("{}.{}", full, m),
            None => full,
        }
    }

    fn test_args(&self, module: &Option<PathBuf>, filter: Option<&str>) -> Vec<String> {
        let gradle_task = self.gradle_task(module, "test");
        let mut args = vec![gradle_task];
        if let Some(f) = filter {
            args.push("--tests".to_string());
            args.push(f.to_string());
        }
        args.extend(Self::debug_args());
        args
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
        let module = self.find_module(file);
        let full_name = full_class_name(package, class, outer);
        let gradle_task = self.gradle_task(&module, "run");

        let mut args = vec![gradle_task, format!("-PmainClass={}", full_name)];
        args.extend(Self::debug_args());

        task(self.command(), args, self.cwd(), vec![])
    }

    fn run_test_method(
        &self,
        file: &str,
        package: &str,
        class: &str,
        outer: Option<&str>,
        method: &str,
    ) -> TaskCommand {
        let module = self.find_module(file);
        let test_filter = Self::test_filter(package, class, outer, Some(method));

        let args = self.test_args(&module, Some(&test_filter));

        task(self.command(), args, self.cwd(), vec![])
    }

    fn run_test_class(
        &self,
        file: &str,
        package: &str,
        class: &str,
        outer: Option<&str>,
    ) -> TaskCommand {
        let module = self.find_module(file);
        let test_filter = Self::test_filter(package, class, outer, None);

        let args = self.test_args(&module, Some(&test_filter));

        task(self.command(), args, self.cwd(), vec![])
    }

    fn run_all_tests(&self, file: &str) -> TaskCommand {
        let module = self.find_module(file);

        let args = self.test_args(&module, None);

        task(self.command(), args, self.cwd(), vec![])
    }
}
