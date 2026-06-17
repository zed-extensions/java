mod build_tool;
mod command;

use crate::build_tool::get_workspace_root;
use std::env;
use std::path::{Path, PathBuf};

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() {
        return;
    }

    let subcommand = &args[0];
    if subcommand == "clear-cache" {
        task_clear_cache();
        return;
    }

    let (tool, _root) = get_workspace_root();

    let result = match subcommand.as_str() {
        "run-class" => {
            if args.len() < 4 {
                return;
            }
            let file = &args[1];
            let package = &args[2];
            let class = &args[3];
            let outer = args.get(4).filter(|s| !s.is_empty()).map(|s| s.as_str());
            Some(tool.run_class(file, package, class, outer))
        }
        "run-test-method" => {
            if args.len() < 6 {
                return;
            }
            let file = &args[1];
            let package = &args[2];
            let class = &args[3];
            let outer = args.get(4).filter(|s| !s.is_empty()).map(|s| s.as_str());
            let method = &args[5];
            Some(tool.run_test_method(file, package, class, outer, method))
        }
        "run-test-class" => {
            if args.len() < 4 {
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
                return;
            }
            let file = &args[1];
            Some(tool.run_all_tests(file))
        }
        _ => None,
    };

    if let Some(cmd) = result {
        // Output JSON for transparency/debugging
        eprintln!("{}", serde_json::to_string(&cmd).unwrap());
        // Execute the task
        cmd.execute();
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
    let cache_dir = if let Ok(xdg) = env::var("XDG_CACHE_HOME") {
        PathBuf::from(xdg)
    } else if cfg!(target_os = "macos") {
        env::var("HOME")
            .map(|h| Path::new(&h).join("Library/Caches"))
            .unwrap_or_default()
    } else {
        env::var("HOME")
            .map(|h| Path::new(&h).join(".cache"))
            .unwrap_or_default()
    };

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
