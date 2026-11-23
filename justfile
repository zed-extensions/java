native_target := `rustc -vV | grep host | awk '{print $2}'`
ext_dir := if os() == "macos" { env("HOME") / "Library/Application Support/Zed/extensions/work/java" } else if os() == "linux" { env("HOME") / ".local/share/zed/extensions/work/java" } else { env("LOCALAPPDATA") / "Zed/extensions/work/java" }
proxy_bin := ext_dir / "bin" / "java-lsp-proxy"
bridge_bin := ext_dir / "bin" / "gradle-lsp-bridge"

# Build proxy in debug mode
proxy-build:
    cargo build --target {{ native_target }} -p java-lsp-proxy

# Build proxy in release mode
proxy-release:
    cargo build --release --target {{ native_target }} -p java-lsp-proxy

# Build proxy release and install to extension workdir for testing
proxy-install: proxy-release
    mkdir -p "{{ ext_dir }}/bin"
    cp "target/{{ native_target }}/release/java-lsp-proxy" "{{ proxy_bin }}"
    @echo "Installed to {{ ext_dir }}"

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
    cargo build --locked --release --target wasm32-wasip2 -p zed_java

# Format all code
fmt:
    cargo fmt --all
    ts_query_ls format languages

# Run clippy on all workspace crates (WASM extension + native binaries)
clippy:
    cargo clippy --workspace --all-targets --fix --allow-dirty

# Format and clippy all code
lint: fmt clippy

# Build everything: lint, extension, and install proxy & bridge
all: lint ext-build proxy-install bridge-install
