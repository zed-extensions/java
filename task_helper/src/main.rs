mod build_tool;
mod command;
mod platform_paths;

use crate::build_tool::get_workspace_root;
use std::env;

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() {
        println!("java-task-helper: no arguments provided");
        return;
    }

    let subcommand = &args[0];
    println!("java-task-helper: running subcommand={}", subcommand);

    if subcommand == "clear-cache" {
        task_clear_cache();
        return;
    }

    let file_path = args.get(1).map(|s| s.as_str());
    let (tool, root) = get_workspace_root(file_path);
    println!(
        "java-task-helper: workspace root={}",
        root.to_string_lossy()
    );

    let result = match subcommand.as_str() {
        "run-class" => {
            if args.len() < 4 {
                eprintln!(
                    "java-task-helper: run-class requires at least 4 arguments, got {}",
                    args.len()
                );
                return;
            }
            let file = &args[1];
            let package = &args[2];
            let class = &args[3];
            let outer = args.get(4).filter(|s| !s.is_empty()).map(|s| s.as_str());
            Some(tool.run_class(file, package, class, outer))
        }
        "run-test-method" => {
            // Accept 5 or 6 args. On Windows, the iex invocation in tasks.json
            // sometimes drops the empty-string outer-class argument, producing
            // 5 positional args: subcommand, file, package, class, method.
            if args.len() == 5 {
                let file = &args[1];
                let package = &args[2];
                let class = &args[3];
                let method = &args[4];
                println!(
                    "java-task-helper: run-test-method file={} package={} class={} method={}",
                    file, package, class, method
                );
                Some(tool.run_test_method(file, package, class, None, method))
            } else if args.len() >= 6 {
                let file = &args[1];
                let package = &args[2];
                let class = &args[3];
                let outer = args.get(4).filter(|s| !s.is_empty()).map(|s| s.as_str());
                let method = &args[5];
                println!(
                    "java-task-helper: run-test-method file={} package={} class={} outer={:?} method={}",
                    file, package, class, outer, method
                );
                Some(tool.run_test_method(file, package, class, outer, method))
            } else {
                eprintln!(
                    "java-task-helper: run-test-method requires 5 or 6 arguments, got {}",
                    args.len()
                );
                return;
            }
        }
        "run-test-class" => {
            if args.len() < 4 {
                eprintln!(
                    "java-task-helper: run-test-class requires at least 4 arguments, got {}",
                    args.len()
                );
                return;
            }
            let file = &args[1];
            let package = &args[2];
            let class = &args[3];
            let outer = args.get(4).filter(|s| !s.is_empty()).map(|s| s.as_str());
            Some(tool.run_test_class(file, package, class, outer))
        }
        "run-all-tests" => {
            if args.len() < 2 {
                eprintln!(
                    "java-task-helper: run-all-tests requires at least 2 arguments, got {}",
                    args.len()
                );
                return;
            }
            let file = &args[1];
            Some(tool.run_all_tests(file))
        }
        _ => {
            eprintln!("java-task-helper: unknown subcommand '{}'", subcommand);
            None
        }
    };

    if let Some(cmd) = result {
        // Output JSON for transparency/debugging
        if let Ok(json) = serde_json::to_string(&cmd) {
            println!("{}", json);
        } else {
            eprintln!("java-task-helper: failed to serialize task command");
        }
        // Execute the task
        println!(
            "java-task-helper: executing command={} args={:?}",
            cmd.command, cmd.args
        );
        cmd.execute();
        println!("java-task-helper: command finished");
    }
}

pub fn is_debug() -> bool {
    env::var("ZED_JAVA_DEBUG").unwrap_or_default() == "1"
}

pub fn get_debug_port() -> String {
    env::var("ZED_JAVA_DEBUG_PORT").unwrap_or_else(|_| "5005".to_string())
}

pub fn get_jdwp_args() -> String {
    format!(
        "-agentlib:jdwp=transport=dt_socket,server=y,suspend=y,address={}",
        get_debug_port()
    )
}

fn task_clear_cache() {
    let cache_dir = platform_paths::get_jdtls_cache_dir();

    if !cache_dir.exists() {
        println!("No JDTLS cache found");
        return;
    }

    let mut cleared = false;
    if let Ok(entries) = std::fs::read_dir(&cache_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name.starts_with("jdtls-") {
                        if let Err(e) = std::fs::remove_dir_all(&path) {
                            eprintln!("Failed to remove cache directory {:?}: {}", path, e);
                        } else {
                            println!("Removed cache directory {:?}", path);
                            cleared = true;
                        }
                    }
                }
            }
        }
    }

    if cleared {
        println!("JDTLS cache cleared. Restart the language server");
    } else {
        println!("No JDTLS cache found");
    }
}
