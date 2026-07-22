use crate::build_tool::{find_closest_module, full_class_name, task, which_wrapper, BuildTool};
use crate::command::TaskCommand;
use crate::{get_jdwp_args, is_debug};
use std::path::PathBuf;

pub struct Maven {
    root: PathBuf,
}

impl Maven {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn find_module(&self, file: &str) -> Option<PathBuf> {
        find_closest_module(file, &self.root, &["pom.xml"])
    }

    fn cwd(&self) -> String {
        self.root.to_string_lossy().to_string()
    }

    fn command(&self) -> String {
        which_wrapper(&self.root, "mvn")
    }

    fn debug_env() -> Vec<(String, String)> {
        if is_debug() {
            vec![("MAVEN_OPTS".to_string(), get_jdwp_args())]
        } else {
            vec![]
        }
    }

    fn debug_arg() -> Option<String> {
        is_debug().then(|| "-Dmaven.surefire.debug".to_string())
    }

    fn application_args(full_name: &str) -> String {
        let mut args = vec![];
        if is_debug() {
            args.push(get_jdwp_args());
        }
        args.extend([
            "-classpath".to_string(),
            "%classpath".to_string(),
            full_name.to_string(),
        ]);
        format!("-Dexec.args={}", args.join(" "))
    }

    fn test_filter(
        package: &str,
        class: &str,
        outer: Option<&str>,
        method: Option<&str>,
    ) -> String {
        let full = full_class_name(package, class, outer);
        match method {
            Some(m) => format!("{}#{}", full, m),
            None => full,
        }
    }

    fn module_prefix(module: &Option<PathBuf>) -> Vec<String> {
        match module {
            Some(m) => vec![
                "-pl".to_string(),
                m.to_string_lossy().to_string(),
                "-am".to_string(),
            ],
            None => vec![],
        }
    }

    fn test_args(module: &Option<PathBuf>) -> Vec<String> {
        let mut args = vec!["test".to_string(), "-U".to_string()];
        args.extend(Self::module_prefix(module));
        args.push("-Dsurefire.failIfNoSpecifiedTests=false".to_string());
        args
    }
}

impl BuildTool for Maven {
    fn run_class(
        &self,
        file: &str,
        package: &str,
        class: &str,
        outer: Option<&str>,
    ) -> TaskCommand {
        let module = self.find_module(file);
        let full_name = full_class_name(package, class, outer);
        let is_test = file.contains("/src/test/");
        let compile_goal = if is_test { "test-compile" } else { "compile" };
        let classpath_scope = if is_test { "test" } else { "runtime" };

        let mut args = vec![compile_goal.to_string(), "exec:exec".to_string()];
        args.extend(Self::module_prefix(&module));
        args.push("-Dexec.executable=java".to_string());
        args.push(Self::application_args(&full_name));
        args.push(format!("-Dexec.classpathScope={}", classpath_scope));
        args.push("-Dexec.inheritIo=true".to_string());
        args.push("-Dexec.longClasspath=true".to_string());

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

        let mut args = Self::test_args(&module);
        args.push(format!("-Dtest={}", test_filter));
        if let Some(a) = Self::debug_arg() {
            args.push(a);
        }

        task(self.command(), args, self.cwd(), Self::debug_env())
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

        let mut args = Self::test_args(&module);
        args.push(format!("-Dtest={}", test_filter));
        if let Some(a) = Self::debug_arg() {
            args.push(a);
        }

        task(self.command(), args, self.cwd(), Self::debug_env())
    }

    fn run_all_tests(&self, file: &str) -> TaskCommand {
        let module = self.find_module(file);

        let mut args = Self::test_args(&module);
        if let Some(a) = Self::debug_arg() {
            args.push(a);
        }

        task(self.command(), args, self.cwd(), Self::debug_env())
    }
}
