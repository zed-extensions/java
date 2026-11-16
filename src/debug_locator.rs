use zed_extension_api::{self as zed, serde_json::json};

const RUNNABLE_ENV: &str = "ZED_JAVA_RUNNABLE";

#[derive(Debug, PartialEq)]
enum JavaRunnable {
    Main,
}

impl JavaRunnable {
    fn from_task(task: &zed::TaskTemplate) -> Option<Self> {
        task.env
            .iter()
            .any(|(name, value)| name == RUNNABLE_ENV && value == "main")
            .then_some(Self::Main)
    }

    fn debug_config(self, debug_adapter_name: &str) -> Option<zed::serde_json::Value> {
        match (self, debug_adapter_name) {
            (Self::Main, "Java") => Some(json!({
                "request": "launch",
                "mainFile": "$ZED_FILE",
            })),
            (Self::Main, "Metals") => Some(json!({
                "request": "launch",
                "path": "$ZED_FILE",
                "runType": "runOrTestFile",
            })),
            _ => None,
        }
    }
}

pub fn create_scenario(
    locator_name: &str,
    build_task: zed::TaskTemplate,
    resolved_label: String,
    debug_adapter_name: &str,
) -> Option<zed::DebugScenario> {
    if locator_name != "java" {
        return None;
    }

    let runnable = JavaRunnable::from_task(&build_task)?;
    let mut config = runnable.debug_config(debug_adapter_name)?;
    let env = build_task
        .env
        .into_iter()
        .filter(|(name, _)| name != RUNNABLE_ENV)
        .map(|(name, value)| (name, value.into()))
        .collect::<zed::serde_json::Map<_, _>>();
    if !env.is_empty() {
        config["env"] = env.into();
    }

    Some(zed::DebugScenario {
        label: resolved_label,
        adapter: debug_adapter_name.to_owned(),
        build: None,
        config: config.to_string(),
        tcp_connection: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use zed::serde_json::{Value, from_str};

    fn main_task() -> zed::TaskTemplate {
        let tasks: Vec<Value> = from_str(include_str!("../languages/java/tasks.json")).unwrap();
        let task = tasks
            .into_iter()
            .find(|task| {
                task["tags"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|tag| tag == "java-main")
            })
            .unwrap();

        zed::TaskTemplate {
            label: task["label"].as_str().unwrap().to_owned(),
            command: task["command"].as_str().unwrap().to_owned(),
            args: Vec::new(),
            env: task["env"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(name, value)| (name.clone(), value.as_str().unwrap().to_owned()))
                .collect(),
            cwd: None,
        }
    }

    #[test]
    fn main_runnable_detection_uses_only_the_task_marker() {
        let mut task = main_task();
        task.label.clear();
        task.command.clear();
        task.args = vec!["custom-argument".into()];
        task.cwd = Some("$ZED_WORKTREE_ROOT/module with spaces".into());
        task.env.push(("APP_MODE".into(), "development".into()));

        assert_eq!(JavaRunnable::from_task(&task), Some(JavaRunnable::Main));
    }

    #[test]
    fn main_runnable_uses_metals_file_discovery_without_starting_a_session() {
        let scenario = create_scenario("java", main_task(), "Run Main".into(), "Metals").unwrap();

        assert_eq!(scenario.label, "Run Main");
        assert_eq!(scenario.adapter, "Metals");
        assert!(scenario.build.is_none());
        assert!(scenario.tcp_connection.is_none());
        assert_eq!(
            from_str::<Value>(&scenario.config).unwrap(),
            json!({
                "request": "launch",
                "path": "$ZED_FILE",
                "runType": "runOrTestFile",
            })
        );
    }

    #[test]
    fn main_runnable_defers_jdtls_resolution_until_launch() {
        let scenario = create_scenario("java", main_task(), "Run Main".into(), "Java").unwrap();

        assert_eq!(scenario.label, "Run Main");
        assert_eq!(scenario.adapter, "Java");
        assert!(scenario.build.is_none());
        assert!(scenario.tcp_connection.is_none());
        assert_eq!(
            from_str::<Value>(&scenario.config).unwrap(),
            json!({
                "request": "launch",
                "mainFile": "$ZED_FILE",
            })
        );
    }

    #[test]
    fn task_environment_is_preserved_without_the_runnable_marker() {
        let mut task = main_task();
        task.env.push(("APP_MODE".into(), "development".into()));
        task.env.push((
            "CONFIG_FILE".into(),
            "$ZED_WORKTREE_ROOT/config.json".into(),
        ));

        for adapter in ["Java", "Metals"] {
            let scenario =
                create_scenario("java", task.clone(), "Run Main".into(), adapter).unwrap();
            let config: Value = from_str(&scenario.config).unwrap();

            assert_eq!(
                config["env"],
                json!({
                    "APP_MODE": "development",
                    "CONFIG_FILE": "$ZED_WORKTREE_ROOT/config.json",
                })
            );
        }
    }

    #[test]
    fn unrelated_tasks_are_not_identified_by_their_labels_or_commands() {
        let mut task = main_task();
        task.env.clear();

        assert_eq!(JavaRunnable::from_task(&task), None);
        for adapter in ["Java", "Metals"] {
            assert!(create_scenario("java", task.clone(), "Run Main".into(), adapter).is_none());
        }
    }

    #[test]
    fn test_runnables_are_not_converted_to_main_sessions() {
        for runnable in ["test-method", "test-class", "test-all"] {
            let mut task = main_task();
            task.env = vec![(RUNNABLE_ENV.into(), runnable.into())];

            assert_eq!(JavaRunnable::from_task(&task), None);
            for adapter in ["Java", "Metals"] {
                assert!(
                    create_scenario("java", task.clone(), "Test Main".into(), adapter).is_none()
                );
            }
        }
    }

    #[test]
    fn unsupported_adapters_do_not_receive_java_configuration() {
        for adapter in ["IntelliJ", "Python", ""] {
            assert!(JavaRunnable::Main.debug_config(adapter).is_none());
            assert!(create_scenario("java", main_task(), "Run Main".into(), adapter).is_none());
        }
    }

    #[test]
    fn other_locators_are_not_intercepted() {
        for adapter in ["Java", "Metals"] {
            assert!(create_scenario("scala", main_task(), "Run Main".into(), adapter).is_none());
        }
    }

    #[test]
    fn main_discovery_does_not_depend_on_package_or_class_captures() {
        let mut task = main_task();
        task.label = "Run current file".into();
        task.command = "custom-runner".into();
        task.cwd = Some("$ZED_WORKTREE_ROOT/module with spaces".into());

        for adapter in ["Java", "Metals"] {
            let scenario =
                create_scenario("java", task.clone(), "Run current file".into(), adapter).unwrap();
            let config: Value = from_str(&scenario.config).unwrap();

            let file_key = if adapter == "Java" {
                "mainFile"
            } else {
                "path"
            };
            assert_eq!(config[file_key], "$ZED_FILE");
            assert!(config.get("mainClass").is_none());
            assert!(config.get("buildTarget").is_none());
        }
    }
}
