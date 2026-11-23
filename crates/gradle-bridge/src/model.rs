//! Converts the Gradle build model into the commands consumed by the Gradle
//! Language Server.

use serde_json::{json, Value};

use crate::proto::gradle::GradleProject;

/// Build the ordered list of `workspace/executeCommand` argument tuples to send
/// to the language server for `root` and every subproject, recursively.
pub fn model_to_commands(root: &GradleProject) -> Vec<(&'static str, Value)> {
    let mut commands = Vec::new();
    collect_commands(root, &mut commands);
    commands
}

fn collect_commands(project: &GradleProject, out: &mut Vec<(&'static str, Value)>) {
    let project_path = normalize_project_path(&project.project_path);

    out.push(("gradle.setPlugins", json!([project_path, project.plugins])));

    let closures: Vec<Value> = project
        .plugin_closures
        .iter()
        .map(|closure| {
            let methods: Vec<Value> = closure
                .methods
                .iter()
                .map(|method| {
                    json!({
                        "name": method.name,
                        "parameterTypes": method.parameter_types,
                        "deprecated": method.deprecated,
                    })
                })
                .collect();
            let fields: Vec<Value> = closure
                .fields
                .iter()
                .map(|field| {
                    json!({
                        "name": field.name,
                        "deprecated": field.deprecated,
                    })
                })
                .collect();
            json!({
                "name": closure.name,
                "methods": methods,
                "fields": fields,
            })
        })
        .collect();
    out.push(("gradle.setClosures", json!([project_path, closures])));

    out.push((
        "gradle.setScriptClasspaths",
        json!([project_path, project.script_classpaths]),
    ));

    for subproject in &project.projects {
        collect_commands(subproject, out);
    }
}

/// Match the key the language server derives from a document URI without
/// resolving symlinks.
fn normalize_project_path(path: &str) -> String {
    use std::path::{Component, PathBuf};

    let mut normalized = PathBuf::new();
    for component in std::path::Path::new(path).components() {
        match component {
            Component::ParentDir => {
                normalized.pop();
            }
            Component::CurDir => {}
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized.to_string_lossy().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::gradle::{GradleClosureProto, GradleFieldProto, GradleMethodProto};

    fn sample_project() -> GradleProject {
        GradleProject {
            is_root: true,
            tasks: vec![],
            projects: vec![GradleProject {
                is_root: false,
                project_path: "/p/sub".to_string(),
                plugins: vec!["java".to_string()],
                ..Default::default()
            }],
            project_path: "/p".to_string(),
            dependency_item: None,
            plugins: vec!["java".to_string(), "application".to_string()],
            plugin_closures: vec![GradleClosureProto {
                name: "java".to_string(),
                methods: vec![GradleMethodProto {
                    name: "sourceCompatibility".to_string(),
                    parameter_types: vec!["String".to_string()],
                    deprecated: false,
                }],
                fields: vec![GradleFieldProto {
                    name: "sourceSets".to_string(),
                    deprecated: false,
                }],
            }],
            script_classpaths: vec!["/p/.gradle/x.jar".to_string()],
        }
    }

    #[test]
    fn emits_three_commands_per_project_recursively() {
        let commands = model_to_commands(&sample_project());
        assert_eq!(commands.len(), 6);
        assert_eq!(commands[0].0, "gradle.setPlugins");
        assert_eq!(commands[1].0, "gradle.setClosures");
        assert_eq!(commands[2].0, "gradle.setScriptClasspaths");
        assert_eq!(commands[0].1[0], "/p");
        assert_eq!(commands[0].1[1][0], "java");
        assert_eq!(commands[3].1[0], "/p/sub");
    }

    #[test]
    fn closure_shape_matches_language_server_contract() {
        let commands = model_to_commands(&sample_project());
        let closures = &commands[1].1[1];
        assert_eq!(closures[0]["name"], "java");
        assert_eq!(closures[0]["methods"][0]["name"], "sourceCompatibility");
        assert_eq!(closures[0]["methods"][0]["parameterTypes"][0], "String");
        assert_eq!(closures[0]["methods"][0]["deprecated"], false);
        assert_eq!(closures[0]["fields"][0]["name"], "sourceSets");
    }

    #[test]
    fn normalizes_dot_segments() {
        assert_eq!(normalize_project_path("/p/./sub/../sub"), "/p/sub");
    }
}
