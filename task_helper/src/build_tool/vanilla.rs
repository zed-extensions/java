use crate::build_tool::BuildTool;
use crate::command::TaskCommand;
use crate::{get_jdwp_args, is_debug};
use std::path::PathBuf;

fn echo_command() -> (String, Vec<String>) {
    if cfg!(windows) {
        (
            "cmd".to_string(),
            vec!["/C".to_string(), "echo".to_string()],
        )
    } else {
        ("echo".to_string(), vec![])
    }
}

pub struct Vanilla {
    root: PathBuf,
}

impl Vanilla {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
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
        let full_class = match outer {
            Some(o) => format!("{}${}", o, class),
            None => class.to_string(),
        };
        let full_name = if package.is_empty() {
            full_class
        } else {
            format!("{}.{}", package, full_class)
        };

        let root = self.root.to_string_lossy().to_string();

        // Java 11+ supports single-file execution: java <file>.java
        // Use it when there's no package and no outer class (simple single class).
        let is_simple = package.is_empty() && outer.is_none();
        if is_simple {
            let mut args = vec![];
            if is_debug() {
                args.push(get_jdwp_args());
            }
            args.push(file.to_string());
            return TaskCommand {
                command: "java".to_string(),
                args,
                cwd: root,
                env: vec![],
                then: vec![],
            };
        }

        let mut run_args = vec!["-cp".to_string(), "bin".to_string()];
        if is_debug() {
            run_args.push(get_jdwp_args());
        }
        run_args.push(full_name);

        TaskCommand {
            command: "javac".to_string(),
            args: vec![
                "-d".to_string(),
                "bin".to_string(),
                "-sourcepath".to_string(),
                ".".to_string(),
                file.to_string(),
            ],
            cwd: root.clone(),
            env: vec![],
            then: vec![TaskCommand {
                command: "java".to_string(),
                args: run_args,
                cwd: root,
                env: vec![],
                then: vec![],
            }],
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
        let (cmd, prefix_args) = echo_command();
        let mut args = prefix_args;
        args.push("No build tool found".to_string());
        TaskCommand {
            command: cmd,
            args,
            cwd: self.root.to_string_lossy().to_string(),
            env: vec![],
            then: vec![],
        }
    }

    fn run_test_class(
        &self,
        _file: &str,
        _package: &str,
        _class: &str,
        _outer: Option<&str>,
    ) -> TaskCommand {
        let (cmd, prefix_args) = echo_command();
        let mut args = prefix_args;
        args.push("No build tool found".to_string());
        TaskCommand {
            command: cmd,
            args,
            cwd: self.root.to_string_lossy().to_string(),
            env: vec![],
            then: vec![],
        }
    }

    fn run_all_tests(&self, _file: &str) -> TaskCommand {
        let (cmd, prefix_args) = echo_command();
        let mut args = prefix_args;
        args.push("No build tool found".to_string());
        TaskCommand {
            command: cmd,
            args,
            cwd: self.root.to_string_lossy().to_string(),
            env: vec![],
            then: vec![],
        }
    }
}
