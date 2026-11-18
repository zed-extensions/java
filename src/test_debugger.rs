use serde::{Deserialize, Serialize};
use zed_extension_api::{
    self as zed,
    serde_json::{self, Value, json},
};

use crate::util::path_to_file_uri;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestTarget {
    pub file: String,
    pub class_name: String,
    #[serde(default)]
    pub outer_class: String,
    pub method: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TestItem {
    full_name: String,
    project_name: String,
    test_level: Value,
    test_kind: Value,
    jdt_handler: Option<String>,
    execution_class_name: Option<String>,
    #[serde(default)]
    children: Vec<TestItem>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestLaunch {
    pub main_class: Option<String>,
    pub project_name: String,
    pub working_directory: String,
    #[serde(default)]
    pub classpath: Vec<String>,
    #[serde(default)]
    pub modulepath: Vec<String>,
    #[serde(default)]
    pub vm_arguments: Vec<String>,
    #[serde(default)]
    pub program_arguments: Vec<String>,
}

impl TestTarget {
    pub fn resolve(
        &self,
        runner_jar: Option<&str>,
        mut request: impl FnMut(&str, Value) -> zed::Result<Value>,
    ) -> zed::Result<TestLaunch> {
        let response = request(
            "vscode.java.test.findTestTypesAndMethods",
            json!([path_to_file_uri(&self.file)]),
        )
        .map_err(|error| format!("Failed to discover Java tests in {}: {error}", self.file))?;
        let items: Vec<TestItem> = serde_json::from_value(response)
            .map_err(|error| format!("Invalid Java test discovery response: {error}"))?;
        let mut classes = Vec::new();
        collect_classes(&items, &mut classes);
        let class_suffix = if self.outer_class.is_empty() {
            self.class_name.clone()
        } else {
            format!("{}.{}", self.outer_class, self.class_name)
        };
        let mut matches = classes.into_iter().filter(|item| {
            let name = item.full_name.replace('$', ".");
            name == class_suffix || name.ends_with(&format!(".{class_suffix}"))
        });
        let class = matches
            .next()
            .ok_or_else(|| format!("No matching test class found in {}", self.file))?;
        if matches.next().is_some() {
            return Err(format!(
                "Multiple matching test classes found in {}",
                self.file
            ));
        }
        let item = if let Some(method) = &self.method {
            let mut methods = class.children.iter().filter(|item| {
                let name = item.full_name.rsplit('#').next().unwrap_or("");
                name.split('(').next() == Some(method.as_str()) && is_level(&item.test_level, 6)
            });
            let item = methods
                .next()
                .ok_or_else(|| format!("No test method {method} found in {}", class.full_name))?;
            if methods.next().is_some() {
                return Err(format!(
                    "Multiple test methods named {method} found in {}; use a manual debug configuration",
                    class.full_name
                ));
            }
            item
        } else {
            class
        };
        if !(0..=3).any(|kind| is_level(&item.test_kind, kind)) {
            return Err(format!("Unsupported Java test kind: {}", item.test_kind));
        }
        let testng = is_level(&item.test_kind, 2);
        let test_name = if self.method.is_some() && !testng {
            item.jdt_handler
                .as_deref()
                .ok_or("Java test discovery did not return a method handle")?
        } else {
            &item.full_name
        };
        let params = json!({
            "projectName": item.project_name,
            "testLevel": item.test_level,
            "testKind": item.test_kind,
            "testNames": [test_name],
            "executionClassNames": [item.execution_class_name],
        });
        let response = request(
            "vscode.java.test.junit.argument",
            json!([params.to_string()]),
        )?;
        if let Some(error) = response.get("errorMessage").and_then(Value::as_str)
            && !error.is_empty()
        {
            return Err(format!(
                "Failed to resolve Java test launch arguments: {error}"
            ));
        }
        let mut launch: TestLaunch = serde_json::from_value(
            response
                .get("body")
                .cloned()
                .ok_or("Java test launch response has no body")?,
        )
        .map_err(|error| format!("Invalid Java test launch arguments: {error}"))?;
        let status = request(
            "vscode.java.buildWorkspace",
            json!([json!({
                "mainClass": class.full_name,
                "projectName": class.project_name,
                "isFullBuild": false,
            })
            .to_string()]),
        )?;
        if status.as_u64() != Some(1) {
            return Err(format!("Java test build did not succeed ({status})"));
        }
        if testng {
            let runner = runner_jar.ok_or("The Java TestNG runner is not installed")?;
            launch.main_class = Some("com.microsoft.java.test.runner.Launcher".into());
            launch.classpath.push(runner.to_owned());
            let methods: Vec<_> = if self.method.is_some() {
                vec![item.full_name.clone()]
            } else {
                class
                    .children
                    .iter()
                    .filter(|item| is_level(&item.test_level, 6))
                    .map(|item| item.full_name.clone())
                    .collect()
            };
            if methods.is_empty() {
                return Err("No TestNG methods found in the selected class".into());
            }
            launch.program_arguments = vec!["testng".into()];
            launch.program_arguments.extend(methods);
        }
        let port = request("zed/javaTestRunner/start", json!({}))?
            .as_u64()
            .filter(|port| *port > 0 && *port <= u16::MAX as u64)
            .ok_or("Invalid Java test runner socket port")?
            .to_string();
        if testng {
            launch.program_arguments.insert(0, port);
        } else if let Some(index) = launch
            .program_arguments
            .iter()
            .rposition(|arg| arg == "-port")
        {
            let value = launch
                .program_arguments
                .get_mut(index + 1)
                .ok_or("JUnit launch arguments contain -port without a value")?;
            *value = port;
        } else {
            launch.program_arguments.extend(["-port".into(), port]);
        }
        Ok(launch)
    }
}

fn is_level(value: &Value, expected: u64) -> bool {
    value.as_u64() == Some(expected)
        || value
            .as_str()
            .is_some_and(|value| value == expected.to_string())
}

fn collect_classes<'a>(items: &'a [TestItem], classes: &mut Vec<&'a TestItem>) {
    for item in items {
        if is_level(&item.test_level, 5) {
            classes.push(item);
        }
        collect_classes(&item.children, classes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(method: Option<&str>) -> TestTarget {
        TestTarget {
            file: "/workspace/module with spaces/src/ExampleTest.java".into(),
            class_name: "ExampleTest".into(),
            outer_class: String::new(),
            method: method.map(str::to_owned),
        }
    }

    fn discovery(kind: &str) -> Value {
        json!([{
            "fullName": "example.ExampleTest",
            "projectName": "module-a",
            "testLevel": "5",
            "testKind": kind,
            "children": [{
                "fullName": "example.ExampleTest#selected(java.lang.String)",
                "projectName": "module-a",
                "testLevel": "6",
                "testKind": kind,
                "jdtHandler": "=module-a/src<example{ExampleTest.java[ExampleTest~selected~QString;",
                "executionClassName": "example.ExampleTest"
            }]
        }])
    }

    fn arguments() -> Value {
        json!({"body": {
            "mainClass": "org.eclipse.jdt.internal.junit.runner.RemoteTestRunner",
            "projectName": "module-a",
            "workingDirectory": "/workspace/module with spaces",
            "classpath": ["test-classes", "junit-runtime.jar"],
            "programArguments": ["-port", "0", "-test", "selected"]
        }})
    }

    #[test]
    fn method_uses_jdt_handle_and_replaces_only_the_result_socket_port() {
        let mut calls = Vec::new();
        let launch = target(Some("selected"))
            .resolve(None, |command, params| {
                calls.push((command.to_owned(), params.clone()));
                Ok(match command {
                    "vscode.java.test.findTestTypesAndMethods" => discovery("0"),
                    "vscode.java.test.junit.argument" => arguments(),
                    "vscode.java.buildWorkspace" => json!(1),
                    "zed/javaTestRunner/start" => json!(12345),
                    _ => panic!("Unexpected command: {command}"),
                })
            })
            .unwrap();

        assert_eq!(
            calls[0].1,
            json!(["file:///workspace/module%20with%20spaces/src/ExampleTest.java"])
        );
        let params: Value = serde_json::from_str(calls[1].1[0].as_str().unwrap()).unwrap();
        assert_eq!(
            params["testNames"][0],
            discovery("0")[0]["children"][0]["jdtHandler"]
        );
        assert_eq!(
            params["executionClassNames"],
            json!(["example.ExampleTest"])
        );
        assert_eq!(
            launch.program_arguments,
            ["-port", "12345", "-test", "selected"]
        );
    }

    #[test]
    fn missing_and_ambiguous_methods_fail_without_starting_a_test_suite() {
        for ambiguous in [false, true] {
            let mut items = discovery("0");
            if ambiguous {
                let duplicate = items[0]["children"][0].clone();
                items[0]["children"].as_array_mut().unwrap().push(duplicate);
            }
            let error = target(Some(if ambiguous { "selected" } else { "missing" }))
                .resolve(None, |command, _| {
                    assert_eq!(command, "vscode.java.test.findTestTypesAndMethods");
                    Ok(items.clone())
                })
                .unwrap_err();

            assert!(error.contains(if ambiguous {
                "Multiple test methods"
            } else {
                "No test method"
            }));
        }
    }

    #[test]
    fn nested_test_class_resolves_without_a_package_variable() {
        let mut target = target(None);
        target.outer_class = "ExampleTest".into();
        target.class_name = "NestedTest".into();
        let mut items = discovery("1");
        items[0]["children"].as_array_mut().unwrap().push(json!({
            "fullName": "example.ExampleTest.NestedTest",
            "projectName": "module-a", "testLevel": 5, "testKind": 1
        }));
        target
            .resolve(None, |command, params| {
                Ok(match command {
                    "vscode.java.test.findTestTypesAndMethods" => items.clone(),
                    "vscode.java.test.junit.argument" => {
                        let params: Value =
                            serde_json::from_str(params[0].as_str().unwrap()).unwrap();
                        assert_eq!(
                            params["testNames"],
                            json!(["example.ExampleTest.NestedTest"])
                        );
                        arguments()
                    }
                    "vscode.java.buildWorkspace" => json!(1),
                    "zed/javaTestRunner/start" => json!(12345),
                    _ => panic!("Unexpected command"),
                })
            })
            .unwrap();
    }

    #[test]
    fn failed_build_never_opens_a_runner_socket() {
        let error = target(None)
            .resolve(None, |command, _| {
                Ok(match command {
                    "vscode.java.test.findTestTypesAndMethods" => discovery("0"),
                    "vscode.java.test.junit.argument" => arguments(),
                    "vscode.java.buildWorkspace" => json!(2),
                    _ => panic!("Runner must not be started after a failed build"),
                })
            })
            .unwrap_err();

        assert!(error.contains("build did not succeed"));
    }

    #[test]
    fn testng_uses_its_runner_and_only_the_selected_method() {
        let launch = target(Some("selected"))
            .resolve(Some("testng-runner.jar"), |command, _| {
                Ok(match command {
                    "vscode.java.test.findTestTypesAndMethods" => discovery("2"),
                    "vscode.java.test.junit.argument" => arguments(),
                    "vscode.java.buildWorkspace" => json!(1),
                    "zed/javaTestRunner/start" => json!(12345),
                    _ => panic!("Unexpected command"),
                })
            })
            .unwrap();

        assert_eq!(
            launch.main_class.as_deref(),
            Some("com.microsoft.java.test.runner.Launcher")
        );
        assert_eq!(launch.classpath.last().unwrap(), "testng-runner.jar");
        assert_eq!(
            launch.program_arguments,
            [
                "12345",
                "testng",
                "example.ExampleTest#selected(java.lang.String)"
            ]
        );
    }
}
