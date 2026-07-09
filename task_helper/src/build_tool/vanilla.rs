use crate::build_tool::{full_class_name, task, BuildTool};
use crate::command::TaskCommand;
use crate::{get_jdwp_args, is_debug};
use std::path::PathBuf;

#[cfg(windows)]
fn echo_command() -> (String, Vec<String>) {
    (
        "cmd".to_string(),
        vec!["/C".to_string(), "echo".to_string()],
    )
}

#[cfg(not(windows))]
fn echo_command() -> (String, Vec<String>) {
    ("echo".to_string(), vec![])
}

pub struct Vanilla {
    root: PathBuf,
}

impl Vanilla {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn cwd(&self) -> String {
        self.root.to_string_lossy().to_string()
    }

    fn no_build_tool(&self) -> TaskCommand {
        let (cmd, mut args) = echo_command();
        args.push("No build tool found".to_string());
        task(cmd, args, self.cwd(), vec![])
    }
}

impl BuildTool for Vanilla {
    fn run_class(
        &self,
        file: &str,
        package: &str,
        class: &str,
        outer: Option<&str>,
    ) -> TaskCommand {
        let full_name = full_class_name(package, class, outer);
        let is_simple = package.is_empty() && outer.is_none();

        if is_simple {
            let mut args = vec![];
            if is_debug() {
                args.push(get_jdwp_args());
            }
            args.push(file.to_string());
            return task("java".to_string(), args, self.cwd(), vec![]);
        }

        let mut run_args = vec!["-cp".to_string(), "bin".to_string()];
        if is_debug() {
            run_args.push(get_jdwp_args());
        }
        run_args.push(full_name);

        let compile = task(
            "javac".to_string(),
            vec![
                "-d".to_string(),
                "bin".to_string(),
                "-sourcepath".to_string(),
                ".".to_string(),
                file.to_string(),
            ],
            self.cwd(),
            vec![],
        );

        let run = task("java".to_string(), run_args, self.cwd(), vec![]);

        TaskCommand {
            then: vec![run],
            ..compile
        }
    }

    fn run_test_method(
        &self,
        _file: &str,
        _package: &str,
        _class: &str,
        _outer: Option<&str>,
        _method: &str,
    ) -> TaskCommand {
        self.no_build_tool()
    }

    fn run_test_class(
        &self,
        _file: &str,
        _package: &str,
        _class: &str,
        _outer: Option<&str>,
    ) -> TaskCommand {
        self.no_build_tool()
    }

    fn run_all_tests(&self, _file: &str) -> TaskCommand {
        self.no_build_tool()
    }
}
