native_target := `rustc -vV | grep host | awk '{print $2}'`
ext_dir := if os() == "macos" { env("HOME") / "Library/Application Support/Zed/extensions/work/java" } else if os() == "linux" { env("HOME") / ".local/share/zed/extensions/work/java" } else { env("LOCALAPPDATA") / "Zed/extensions/work/java" }
proxy_bin := ext_dir / "bin" / "java-lsp-proxy"
tasks_bin := ext_dir / "bin" / "java-task-helper"
bridge_bin := ext_dir / "bin" / "gradle-lsp-bridge"

# Build proxy in debug mode
proxy-build:
    cargo build --target {{ native_target }} -p java-lsp-proxy

# Build proxy in release mode
proxy-release:
    cd proxy && cargo build --release --target {{ native_target }}

# Build proxy release and install to extension workdir for testing
proxy-install: proxy-release
    mkdir -p "{{ ext_dir }}/bin"
    cp "target/{{ native_target }}/release/java-lsp-proxy" "{{ proxy_bin }}"
    @echo "Installed to {{ ext_dir }}"

# --- Task helper recipes ---

# Build task helper in debug mode
task-build:
    cd task_helper && cargo build --target {{ native_target }}

# Build task helper in release mode
task-release:
    cd task_helper && cargo build --release --target {{ native_target }}

# Build task helper release and install to extension workdir for testing
task-install: task-release
    mkdir -p "{{ ext_dir }}/bin"
    cp "target/{{ native_target }}/release/java-task-helper" "{{ tasks_bin }}"
    @echo "Installed to {{ ext_dir }}"

# Run task helper tests
task-test:
    cd task_helper && cargo test

# Clean task helper build
task-clean:
    cd task_helper && cargo clean

# --- Core recipes ---
# Build gradle-lsp-bridge in debug mode
bridge-build:
    cargo build --target {{ native_target }} -p gradle-lsp-bridge

# Build gradle-lsp-bridge in release mode
bridge-release:
    cargo build --release --target {{ native_target }} -p gradle-lsp-bridge

# Build gradle-lsp-bridge release and install to extension workdir for testing
bridge-install: bridge-release
    mkdir -p "{{ ext_dir }}/bin"
    cp "target/{{ native_target }}/release/gradle-lsp-bridge" "{{ bridge_bin }}"
    @echo "Installed to {{ ext_dir }}"

# Build WASM extension in release mode
ext-build:
    cargo build --release

# Format all code
fmt:
    cargo fmt --all
    ts_query_ls format languages

# Run clippy on all workspace crates (WASM extension + native binaries)
clippy:
    cargo clippy --workspace --all-targets --fix --allow-dirty

# Format and lint all code
lint: fmt clippy

# Build everything: lint, extension, and install proxy, task helper & bridge
all: lint ext-build proxy-install task-install bridge-install
