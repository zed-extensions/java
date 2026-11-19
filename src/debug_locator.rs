use zed_extension_api::{self as zed, serde_json::json};

const RUNNABLE_ENV: &str = "ZED_JAVA_RUNNABLE";
const MAIN_CLASS_ENV: &str = "ZED_JAVA_MAIN_CLASS";

#[derive(Clone, Copy, Debug, PartialEq)]
enum JavaRunnable {
    Main,
    TestMethod,
    TestClass,
}

impl JavaRunnable {
    fn from_task(task: &zed::TaskTemplate) -> Option<Self> {
        match task
            .env
            .iter()
            .find(|(name, _)| name == RUNNABLE_ENV)
            .map(|(_, value)| value.as_str())
        {
            Some("main") => Some(Self::Main),
            Some("test-method") => Some(Self::TestMethod),
            Some("test-class") => Some(Self::TestClass),
            _ => None,
        }
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
            (Self::TestMethod | Self::TestClass, "Java") => {
                let mut target = json!({
                    "file": "$ZED_FILE",
                    "className": "$ZED_CUSTOM_java_class_name",
                    "outerClass": "${ZED_CUSTOM_java_outer_class_name:}",
                });
                if self == Self::TestMethod {
                    target["method"] = "$ZED_CUSTOM_java_method_name".into();
                }
                Some(json!({
                    "request": "launch",
                    "testTarget": target,
                }))
            }
            _ => None,
        }
    }
}

pub fn create_scenario(
    build_task: zed::TaskTemplate,
    resolved_label: String,
    debug_adapter_name: &str,
) -> Option<zed::DebugScenario> {
    let runnable = JavaRunnable::from_task(&build_task)?;
    let mut config = runnable.debug_config(debug_adapter_name)?;
    if runnable == JavaRunnable::Main
        && debug_adapter_name == "Java"
        && let Some((_, main_class)) = build_task
            .env
            .iter()
            .find(|(name, value)| name == MAIN_CLASS_ENV && !value.is_empty())
    {
        config["mainClass"] = main_class.as_str().into();
    }
    let env = build_task
        .env
        .into_iter()
        .filter(|(name, _)| name != RUNNABLE_ENV && name != MAIN_CLASS_ENV)
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
        let scenario = create_scenario(main_task(), "Run Main".into(), "Metals").unwrap();

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
        let scenario = create_scenario(main_task(), "Run Main".into(), "Java").unwrap();

        assert_eq!(scenario.label, "Run Main");
        assert_eq!(scenario.adapter, "Java");
        assert!(scenario.build.is_none());
        assert!(scenario.tcp_connection.is_none());
        assert_eq!(
            from_str::<Value>(&scenario.config).unwrap(),
            json!({
                "request": "launch",
                "mainFile": "$ZED_FILE",
                "mainClass": "$ZED_CUSTOM_java_class_name",
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
            let scenario = create_scenario(task.clone(), "Run Main".into(), adapter).unwrap();
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
            assert!(create_scenario(task.clone(), "Run Main".into(), adapter).is_none());
        }
    }

    #[test]
    fn test_runnables_are_not_converted_to_main_sessions() {
        for runnable in ["test-method", "test-class"] {
            let mut task = main_task();
            task.env = vec![(RUNNABLE_ENV.into(), runnable.into())];

            {
                let scenario = create_scenario(task.clone(), "Debug test".into(), "Java").unwrap();
                let config: Value = from_str(&scenario.config).unwrap();
                assert_eq!(config["testTarget"]["file"], "$ZED_FILE");
                assert_eq!(
                    config["testTarget"]["className"],
                    "$ZED_CUSTOM_java_class_name"
                );
                assert!(config.get("mainFile").is_none());
                assert!(config.get("path").is_none());
                assert_eq!(
                    config["testTarget"].get("method").is_some(),
                    runnable == "test-method"
                );
            }
            assert!(create_scenario(task, "Debug test".into(), "Metals").is_none());
        }
    }

    #[test]
    fn whole_project_test_tasks_are_not_converted() {
        let mut task = main_task();
        task.env = vec![(RUNNABLE_ENV.into(), "test-all".into())];

        for adapter in ["Java", "Metals"] {
            assert!(create_scenario(task.clone(), "Run tests".into(), adapter).is_none());
        }
    }

    #[test]
    fn unsupported_adapters_do_not_receive_java_configuration() {
        for adapter in ["IntelliJ", "Python", ""] {
            assert!(JavaRunnable::Main.debug_config(adapter).is_none());
            assert!(create_scenario(main_task(), "Run Main".into(), adapter).is_none());
        }
    }

    #[test]
    fn main_discovery_does_not_depend_on_package_or_class_captures() {
        let mut task = main_task();
        task.label = "Run current file".into();
        task.command = "custom-runner".into();
        task.cwd = Some("$ZED_WORKTREE_ROOT/module with spaces".into());
        task.env.retain(|(name, _)| name != MAIN_CLASS_ENV);

        for adapter in ["Java", "Metals"] {
            let scenario =
                create_scenario(task.clone(), "Run current file".into(), adapter).unwrap();
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

    #[test]
    fn explicit_class_hint_is_forwarded_without_exposing_it_as_environment() {
        let mut task = main_task();
        task.env = vec![
            (RUNNABLE_ENV.into(), "main".into()),
            (MAIN_CLASS_ENV.into(), "example.Other".into()),
        ];

        let scenario = create_scenario(task, "Run Other".into(), "Java").unwrap();
        let config: Value = from_str(&scenario.config).unwrap();

        assert_eq!(config["mainClass"], "example.Other");
        assert!(config.get("env").is_none());
    }

    #[test]
    fn empty_class_hint_keeps_file_discovery() {
        let mut task = main_task();
        task.env = vec![
            (RUNNABLE_ENV.into(), "main".into()),
            (MAIN_CLASS_ENV.into(), String::new()),
        ];

        let scenario = create_scenario(task, "Run current file".into(), "Java").unwrap();
        let config: Value = from_str(&scenario.config).unwrap();

        assert_eq!(config["mainFile"], "$ZED_FILE");
        assert!(config.get("mainClass").is_none());
        assert!(config.get("env").is_none());
    }
}
