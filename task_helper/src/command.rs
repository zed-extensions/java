use serde::Serialize;
use std::process::{self, Command};

#[derive(Serialize)]
pub struct TaskCommand {
    pub command: String,
    pub args: Vec<String>,
    pub cwd: String,
}

impl TaskCommand {
    pub fn execute(self) {
        let mut cmd = Command::new(&self.command);
        cmd.args(&self.args);
        cmd.current_dir(&self.cwd);

        // Inherit stdin/stdout/stderr
        cmd.stdin(process::Stdio::inherit());
        cmd.stdout(process::Stdio::inherit());
        cmd.stderr(process::Stdio::inherit());

        let mut child = cmd.spawn().unwrap_or_else(|e| {
            eprintln!("Failed to execute {}: {}", self.command, e);
            process::exit(1);
        });

        let status = child.wait().unwrap_or_else(|e| {
            eprintln!("Failed to wait for {}: {}", self.command, e);
            process::exit(1);
        });

        process::exit(status.code().unwrap_or(0));
    }
}
