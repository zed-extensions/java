use crate::build_tool::BuildTool;
use crate::command::TaskCommand;
use crate::{get_jdwp_args, is_debug};
use std::path::PathBuf;

fn echo_command() -> (String, Vec<String>) {
    // On Windows, `echo` is a cmd.exe built-in, not an executable.
    // Use `cmd /c echo` so Command::new succeeds.
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
        _file: &str,
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

        let debug_args = if is_debug() {
            format!("{} ", get_jdwp_args())
        } else {
            "".to_string()
        };

        TaskCommand {
            command: "sh".to_string(),
            args: vec![
                "-c".to_string(),
                format!("find . -name '*.java' -not -path './bin/*' -not -path './target/*' -not -path './build/*' -print0 | xargs -0 javac -d bin && java {} -cp bin \"{}\"", debug_args, full_name),
            ],
            cwd: self.root.to_string_lossy().to_string(),
            env: vec![],
            then: vec![],
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
