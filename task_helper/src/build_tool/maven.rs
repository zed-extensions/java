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

    fn debug_env() -> Vec<(String, String)> {
        if is_debug() {
            vec![("MAVEN_OPTS".to_string(), get_jdwp_args())]
        } else {
            vec![]
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
        let env = Self::debug_env();

        if let Some(m) = module {
            let m_str = m.to_string_lossy().to_string();
            TaskCommand {
                command,
                args: vec![
                    "clean".to_string(),
                    compile_goal.to_string(),
                    "exec:java".to_string(),
                    "-pl".to_string(),
                    m_str,
                    "-am".to_string(),
                    format!("-Dexec.mainClass={}", full_name),
                    format!("-Dexec.classpathScope={}", classpath_scope),
                ],
                cwd: self.root.to_string_lossy().to_string(),
                env,
                then: vec![],
            }
        } else {
            TaskCommand {
                command,
                args: vec![
                    "clean".to_string(),
                    compile_goal.to_string(),
                    "exec:java".to_string(),
                    format!("-Dexec.mainClass={}", full_name),
                    format!("-Dexec.classpathScope={}", classpath_scope),
                ],
                cwd: self.root.to_string_lossy().to_string(),
                env,
                then: vec![],
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
        let env = Self::debug_env();

        if let Some(m) = module {
            let m_str = m.to_string_lossy().to_string();
            let mut args = vec![
                "clean".to_string(),
                "test".to_string(),
                "-pl".to_string(),
                m_str,
                "-am".to_string(),
                "-Dsurefire.failIfNoSpecifiedTests=false".to_string(),
                format!("-Dtest={}", test_filter),
            ];
            if is_debug() {
                args.push("-Dmaven.surefire.debug".to_string());
            }
            TaskCommand {
                command,
                args,
                cwd: self.root.to_string_lossy().to_string(),
                env,
                then: vec![],
            }
        } else {
            let mut args = vec![
                "clean".to_string(),
                "test".to_string(),
                format!("-Dtest={}", test_filter),
            ];
            if is_debug() {
                args.push("-Dmaven.surefire.debug".to_string());
            }
            TaskCommand {
                command,
                args,
                cwd: self.root.to_string_lossy().to_string(),
                env,
                then: vec![],
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
        let env = Self::debug_env();

        if let Some(m) = module {
            let m_str = m.to_string_lossy().to_string();
            let mut args = vec![
                "clean".to_string(),
                "test".to_string(),
                "-pl".to_string(),
                m_str,
                "-am".to_string(),
                "-Dsurefire.failIfNoSpecifiedTests=false".to_string(),
                format!("-Dtest={}", test_filter),
            ];
            if is_debug() {
                args.push("-Dmaven.surefire.debug".to_string());
            }
            TaskCommand {
                command,
                args,
                cwd: self.root.to_string_lossy().to_string(),
                env,
                then: vec![],
            }
        } else {
            let mut args = vec![
                "clean".to_string(),
                "test".to_string(),
                format!("-Dtest={}", test_filter),
            ];
            if is_debug() {
                args.push("-Dmaven.surefire.debug".to_string());
            }
            TaskCommand {
                command,
                args,
                cwd: self.root.to_string_lossy().to_string(),
                env,
                then: vec![],
            }
        }
    }

    fn run_all_tests(&self, file: &str) -> TaskCommand {
        let command = which_wrapper(&self.root, "mvn");
        let module = self.find_module(file);
        let env = Self::debug_env();

        if let Some(m) = module {
            let m_str = m.to_string_lossy().to_string();
            let mut args = vec![
                "clean".to_string(),
                "test".to_string(),
                "-pl".to_string(),
                m_str,
                "-am".to_string(),
            ];
            if is_debug() {
                args.push("-Dmaven.surefire.debug".to_string());
            }
            TaskCommand {
                command,
                args,
                cwd: self.root.to_string_lossy().to_string(),
                env,
                then: vec![],
            }
        } else {
            let mut args = vec!["clean".to_string(), "test".to_string()];
            if is_debug() {
                args.push("-Dmaven.surefire.debug".to_string());
            }
            TaskCommand {
                command,
                args,
                cwd: self.root.to_string_lossy().to_string(),
                env,
                then: vec![],
            }
        }
    }
}
