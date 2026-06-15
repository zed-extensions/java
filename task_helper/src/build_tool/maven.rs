use crate::build_tool::{find_closest_module, which_wrapper, BuildTool};
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

    fn full_class_name(&self, package: &str, class: &str, outer: Option<&str>) -> String {
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
}

impl BuildTool for Maven {
    fn run_class(
        &self,
        file: &str,
        package: &str,
        class: &str,
        outer: Option<&str>,
    ) -> TaskCommand {
        let command = which_wrapper(&self.root, "mvn");
        let module = self.find_module(file);
        let full_name = self.full_class_name(package, class, outer);
        let is_test = file.contains("/src/test/");
        let compile_goal = if is_test { "test-compile" } else { "compile" };
        let classpath_scope = if is_test { "test" } else { "runtime" };

        let env_prefix = if is_debug() {
            format!("MAVEN_OPTS=\"{}\" ", get_jdwp_args())
        } else {
            "".to_string()
        };

        if let Some(m) = module {
            let m_str = m.to_string_lossy().to_string();
            let shell_cmd = format!(
                "{}{} clean {} -pl \"{}\" -am && {}{} exec:java -pl \"{}\" -Dexec.mainClass=\"{}\" -Dexec.classpathScope={}",
                env_prefix, command, compile_goal, m_str, env_prefix, command, m_str, full_name, classpath_scope
            );
            TaskCommand {
                command: "sh".to_string(),
                args: vec!["-c".to_string(), shell_cmd],
                cwd: self.root.to_string_lossy().to_string(),
            }
        } else {
            let shell_cmd = format!(
                "{}{} clean {} exec:java -Dexec.mainClass=\"{}\" -Dexec.classpathScope={}",
                env_prefix, command, compile_goal, full_name, classpath_scope
            );
            TaskCommand {
                command: "sh".to_string(),
                args: vec!["-c".to_string(), shell_cmd],
                cwd: self.root.to_string_lossy().to_string(),
            }
        }
    }

    fn run_test_method(
        &self,
        file: &str,
        package: &str,
        class: &str,
        outer: Option<&str>,
        method: &str,
    ) -> TaskCommand {
        let command = which_wrapper(&self.root, "mvn");
        let module = self.find_module(file);
        let full_class = match outer {
            Some(o) => format!("{}${}", o, class),
            None => class.to_string(),
        };
        let test_filter = if package.is_empty() {
            format!("{}#{}", full_class, method)
        } else {
            format!("{}.{}#{}", package, full_class, method)
        };

        let debug_arg = if is_debug() {
            " -Dmaven.surefire.debug"
        } else {
            ""
        };

        if let Some(m) = module {
            let m_str = m.to_string_lossy().to_string();
            let shell_cmd = format!(
                "{} clean test-compile -pl \"{}\" -am && {} test -pl \"{}\" -Dtest='{}'{}",
                command, m_str, command, m_str, test_filter, debug_arg
            );
            TaskCommand {
                command: "sh".to_string(),
                args: vec!["-c".to_string(), shell_cmd],
                cwd: self.root.to_string_lossy().to_string(),
            }
        } else {
            let shell_cmd = format!(
                "{} clean test -Dtest='{}'{}",
                command, test_filter, debug_arg
            );
            TaskCommand {
                command: "sh".to_string(),
                args: vec!["-c".to_string(), shell_cmd],
                cwd: self.root.to_string_lossy().to_string(),
            }
        }
    }

    fn run_test_class(
        &self,
        file: &str,
        package: &str,
        class: &str,
        outer: Option<&str>,
    ) -> TaskCommand {
        let command = which_wrapper(&self.root, "mvn");
        let module = self.find_module(file);
        let full_class = match outer {
            Some(o) => format!("{}${}", o, class),
            None => class.to_string(),
        };
        let test_filter = if package.is_empty() {
            full_class
        } else {
            format!("{}.{}", package, full_class)
        };

        let debug_arg = if is_debug() {
            " -Dmaven.surefire.debug"
        } else {
            ""
        };

        if let Some(m) = module {
            let m_str = m.to_string_lossy().to_string();
            let shell_cmd = format!(
                "{} clean test-compile -pl \"{}\" -am && {} test -pl \"{}\" -Dtest='{}'{}",
                command, m_str, command, m_str, test_filter, debug_arg
            );
            TaskCommand {
                command: "sh".to_string(),
                args: vec!["-c".to_string(), shell_cmd],
                cwd: self.root.to_string_lossy().to_string(),
            }
        } else {
            let shell_cmd = format!(
                "{} clean test -Dtest='{}'{}",
                command, test_filter, debug_arg
            );
            TaskCommand {
                command: "sh".to_string(),
                args: vec!["-c".to_string(), shell_cmd],
                cwd: self.root.to_string_lossy().to_string(),
            }
        }
    }

    fn run_all_tests(&self, file: &str) -> TaskCommand {
        let command = which_wrapper(&self.root, "mvn");
        let module = self.find_module(file);
        let debug_arg = if is_debug() {
            " -Dmaven.surefire.debug"
        } else {
            ""
        };

        if let Some(m) = module {
            let m_str = m.to_string_lossy().to_string();
            let shell_cmd = format!(
                "{} clean test-compile -pl \"{}\" -am && {} test -pl \"{}\"{}",
                command, m_str, command, m_str, debug_arg
            );
            TaskCommand {
                command: "sh".to_string(),
                args: vec!["-c".to_string(), shell_cmd],
                cwd: self.root.to_string_lossy().to_string(),
            }
        } else {
            let shell_cmd = format!("{} clean test{}", command, debug_arg);
            TaskCommand {
                command: "sh".to_string(),
                args: vec!["-c".to_string(), shell_cmd],
                cwd: self.root.to_string_lossy().to_string(),
            }
        }
    }
}
