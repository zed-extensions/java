native_target := `rustc -vV | grep host | awk '{print $2}'`

ext_dir := if os() == "macos" {
    env("HOME") / "Library/Application Support/Zed/extensions/work/java"
} else if os() == "linux" {
    env("HOME") / ".local/share/zed/extensions/work/java"
} else {
    env("LOCALAPPDATA") / "Zed/extensions/work/java"
}

proxy_bin := ext_dir / "proxy-bin" / "java-lsp-proxy"

# Build proxy in debug mode
proxy-build:
    cd proxy && cargo build --target {{native_target}}

# Build proxy in release mode
proxy-release:
    cd proxy && cargo build --release --target {{native_target}}

# Build proxy release and install to extension workdir for testing
proxy-install: proxy-release
    mkdir -p "{{ext_dir}}/proxy-bin"
    cp "proxy/target/{{native_target}}/release/java-lsp-proxy" "{{proxy_bin}}"
    @echo "Installed to {{proxy_bin}}"

# Build WASM extension in release mode
ext-build:
    cargo build --release

# Format all code
fmt:
    cargo fmt --all
    cd proxy && cargo fmt --all

# Run clippy on both crates
clippy:
    cargo clippy --all-targets --fix --allow-dirty
    cd proxy && cargo clippy --all-targets --fix --allow-dirty --target {{native_target}}

# Build everything: fmt, clippy, extension, proxy install
all: fmt clippy ext-build proxy-install
