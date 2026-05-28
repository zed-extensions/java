# Java Extension for Zed

This extension adds support for Java and `.properties` files to [Zed](https://zed.dev). It uses the [Eclipse JDT Language Server](https://projects.eclipse.org/projects/eclipse.jdt.ls) (JDTLS for short) to provide completions, code-actions and diagnostics.

## Quick Start

Install the extension via Zeds extension manager. It should work out of the box for most people. However, there are some things to know:

- It is generally recommended to open projects with the Zed-project root at the Java project root folder (where you would commonly have your `pom.xml` or `build.gradle` file). The extension will automatically detect Maven and Gradle projects in subdirectories, but opening at the project root provides the best experience. If you're working with a non-standard project layout or encounter issues with classpath resolution, see [Advanced Configuration/JDTLS initialization Options](#advanced-configurationjdtls-initialization-options) for fine-tuning.

- By default the extension will download and run the latest official version of JDTLS for you, but this requires Java version 21 or higher to be available on your system via either the `$JAVA_HOME` environment variable or as a `java(.exe)` executable on your `$PATH`. The `java_home` configuration option allows you to specify a **separate** JDK 21+ installation specifically for running JDTLS — this is useful when your system default Java is a lower version required by your project. Note that `java_home` is also passed as the `JAVA_HOME` environment variable to the JDTLS process; to configure runtimes for your project itself, use `initialization_options.settings.java.configuration.runtimes` (see [Configuring Project Runtimes](#configuring-project-runtimes) below).

- You can provide a **custom JDTLS binary** through one of these mechanisms (in priority order):
  1. The `jdtls_launcher` setting — specify an absolute path to a JDTLS launch script
  2. An executable named `jdtls` (or `jdtls.bat` on Windows) on your `$PATH`
  
  When either is found, the extension will skip downloading and launching a managed JDTLS instance and use the provided one instead.

- To support [Lombok](https://projectlombok.org/), the lombok-jar must be downloaded and registered as a Java-Agent when launching JDTLS. By default the extension automatically takes care of that, but in case you don't want that you can set the `lombok_support` configuration-option to `false`.

- The option to let the extension automatically download a JDK can be enabled by setting `jdk_auto_download` to `true`. When enabled, the extension will download [Amazon Corretto](https://aws.amazon.com/corretto/) (an OpenJDK distribution) if no valid `java_home` is provided or if the specified one does not meet the minimum version requirement (Java 21). User-provided JDKs **always** take precedence.

Here is a common `settings.json` including the above mentioned configurations:

```jsonc
"lsp": {
 "jdtls": {
    "settings": {
      // Path to a JDK 21+ used to run JDTLS.
      // Also accepts the legacy key "java.home".
      "java_home": "/path/to/your/JDK21+",
      "lombok_support": true,
      "jdk_auto_download": false,

      // JVM heap size for JDTLS (maps to -Xms and -Xmx)
      // Accepts values like "512m", "1G", "4096m", etc.
      "min_memory": "1G",   // default: "1G"
      "max_memory": "2G",   // default: unset (no -Xmx limit)

      // Controls when to check for updates for JDTLS, Lombok, and Debugger
      // - "always" (default): Always check for and download the latest version
      // - "once": Check for updates only if no local installation exists
      // - "never": Never check for updates, only use existing local installations (errors if missing)
      //
      // Note: Invalid values will default to "always"
      // If custom paths (below) are provided, check_updates is IGNORED for that component
      "check_updates": "always",
      
      // Use custom installations instead of managed downloads
      // When these are set, the extension will not download or manage these components
      "jdtls_launcher": "/path/to/your/jdt-language-server/bin/jdtls",
      "lombok_jar": "/path/to/your/lombok.jar",
      "java_debug_jar": "/path/to/your/com.microsoft.java.debug.plugin.jar",
      "lsp_proxy_path": "/path/to/your/java-lsp-proxy"
    }
  }
}
```

## Project Symbol Search

The extension supports project-wide symbol search with syntax-highlighted results. This feature is powered by JDTLS and can be accessed via Zed's symbol search.

JDTLS uses **CamelCase fuzzy matching** for symbol queries. For example, searching for `EmpMe` would match `EmptyMedia`. The pattern works like `Emp*Me*`, matching the capital letters of CamelCase names.

## Debugger

Debug support is enabled via our [Fork of Java Debug](https://github.com/zed-industries/java-debug), which the extension will automatically download and start for you. Please refer to the [Zed Documentation](https://zed.dev/docs/debugger#getting-started) for general information about how debugging works in Zed.

### Launch Mode

To get started with Java, click the `edit debug.json` button in the Debug menu, and replace the contents of the file with the following:
```jsonc
[
  {
    "adapter": "Java",
    "request": "launch",
    "label": "Launch Debugger",
    // if your project has multiple entry points, specify the one to use:
    // "mainClass": "com.myorganization.myproject.MyMainClass",
    //
    // this effectively sets a breakpoint at your program entry:
    "stopOnEntry": true,
    // the working directory for the debug process
    "cwd": "$ZED_WORKTREE_ROOT"
  }
]
```

You should then be able to start a new Debug Session with the "Launch Debugger" scenario from the debug menu.

### Attach Mode

You can attach to a running JVM process that was started with debug options (e.g. `-agentlib:jdwp=transport=dt_socket,server=y,suspend=n,address=5005`):

```jsonc
[
  {
    "label": "Attach to JVM (port 5005)",
    "adapter": "Java",
    "request": "attach",
    "hostName": "localhost",
    "port": 5005
  }
]
```

### Debug Configuration Reference

The following options are available in `debug.json` for **launch** configurations:

| Option | Type | Description |
|--------|------|-------------|
| `request` | `"launch"` | Required. The request type. |
| `mainClass` | `string` | Fully qualified class name. Auto-resolved if omitted. |
| `projectName` | `string` | Project name to disambiguate when multiple projects exist. |
| `args` | `string \| string[]` | Command line arguments passed to the program. |
| `vmArgs` | `string \| string[]` | Extra JVM options (e.g. `-Xmx2G -Dprop=value`). |
| `classPaths` | `string[]` | Classpaths for the JVM. Special values: `$Auto`, `$Runtime`, `$Test`. |
| `modulePaths` | `string[]` | Module paths for the JVM. Auto-resolved if omitted. |
| `cwd` | `string` | Working directory. Defaults to the worktree root. |
| `env` | `object` | Extra environment variables for the program. |
| `encoding` | `string` | The `file.encoding` setting for the JVM. |
| `stopOnEntry` | `boolean` | Pause the program after launching. |
| `noDebug` | `boolean` | Launch without attaching the debugger (e.g. for profiling). |
| `console` | `string` | Console type: `internalConsole`, `integratedTerminal`, or `externalTerminal`. |
| `shortenCommandLine` | `string` | Shorten long command lines: `none`, `jarmanifest`, or `argfile`. |
| `launcherScript` | `string` | Path to a custom JVM launcher script. |
| `javaExec` | `string` | Path to a specific Java executable. |

For **attach** configurations:

| Option | Type | Description |
|--------|------|-------------|
| `request` | `"attach"` | Required. The request type. |
| `hostName` | `string` | Host name or IP of the remote debuggee. |
| `port` | `integer` | Debug port of the remote debuggee. |
| `timeout` | `integer` | Timeout before reconnecting in milliseconds (default: 30000). |
| `projectName` | `string` | Project name for source resolution. |

### Single-File Debugging

If you're working a lot with single file debugging, you can use the following `debug.json` config instead:
```jsonc
[
  {
    "label": "Debug $ZED_STEM",
    "adapter": "Java",
    "request": "launch",
    "mainClass": "$ZED_STEM",
    "build": {
      "command": "javac -d . $ZED_FILE",
      "shell": {
        "with_arguments": {
          "program": "/bin/sh",
          "args": ["-c"]
        }
      }
    }
  }
]
```
This will compile and launch the debugger using the currently selected file as the entry point. 
Ideally, we would implement a run/debug option directly in the runnables (similar to how the Rust extension does it), which would allow you to easily start a debugging session without explicitly updating the entry point.
Note that integrating the debugger with runnables is currently limited to core languages in Zed, so this is the best workaround for now. 

## Launch Scripts (aka Tasks) in Windows

This extension provides tasks for running your application and tests from within Zed via little play buttons next to tests/entry points. However, due to current limitiations of Zed's extension interface, we can not provide scripts that will work across Maven and Gradle on both Windows and Unix-compatible systems, so out of the box the launch scripts only work on Mac and Linux.

There is a fairly straightforward fix that you can apply to make it work on Windows by supplying your own task scripts. Please see [this Issue](https://github.com/zed-extensions/java/issues/94) for information on how to do that and read the [Tasks section in Zeds documentation](https://zed.dev/docs/tasks) for more information.

## Configuring Project Runtimes

If your project targets a Java version different from the one running JDTLS, you can register multiple JDK installations via `java.configuration.runtimes`. JDTLS will use these to compile and run your project at the correct language level, while still running itself on JDK 21+.

```jsonc
"lsp": {
  "jdtls": {
    "settings": {
      // JDK 21+ for running JDTLS itself
      "java_home": "/usr/lib/jvm/java-21-openjdk"
    },
    "initialization_options": {
      "settings": {
        "java": {
          "configuration": {
            "runtimes": [
              {
                "name": "JavaSE-1.8",
                "path": "/usr/lib/jvm/java-8-openjdk"
              },
              {
                "name": "JavaSE-11",
                "path": "/usr/lib/jvm/java-11-openjdk"
              },
              {
                "name": "JavaSE-17",
                "path": "/usr/lib/jvm/java-17-openjdk"
              },
              {
                "name": "JavaSE-21",
                "path": "/usr/lib/jvm/java-21-openjdk",
                "default": true
              }
            ]
          }
        }
      }
    }
  }
}
```

- `name` must match an [execution environment identifier](https://wiki.eclipse.org/Execution_Environments) (e.g. `JavaSE-1.8`, `JavaSE-11`, `JavaSE-17`, `JavaSE-21`). Note that Java 8 uses the `1.8` naming convention while Java 9+ uses the major version number directly.
- `path` is the absolute path to the JDK installation root (the directory containing `bin/java`).
- `default` (optional) — set to `true` on the runtime JDTLS should use when no project-specific source level is detected.

JDTLS will automatically pick the appropriate runtime based on your project's source level (from `pom.xml`, `build.gradle`, or `.classpath`). For example, a Maven project with `<maven.compiler.source>11</maven.compiler.source>` will use the `JavaSE-11` runtime for compilation and code analysis.

> **macOS paths** typically look like `/Library/Java/JavaVirtualMachines/jdk-17.jdk/Contents/Home`
>
> **Windows paths** typically look like `C:\Program Files\Java\jdk-17`

## Advanced Configuration/JDTLS initialization Options

JDTLS provides many configuration options that can be passed via the `initialize` LSP-request. The extension will pass the JSON-object from `lsp.jdtls.initialization_options` in your settings on to JDTLS. Please refer to the [JDTLS Configuration Wiki Page](https://github.com/eclipse-jdtls/eclipse.jdt.ls/wiki/Running-the-JAVA-LS-server-from-the-command-line#initialize-request) for the available options and values.

The extension automatically injects the following defaults into `initialization_options` (unless you override them):
- `workspaceFolders` — set to the worktree root as a `file://` URI
- `extendedClientCapabilities.classFileContentsSupport` — `true` (enables decompiled source navigation)
- `extendedClientCapabilities.resolveAdditionalTextEditsSupport` — `true`

Below is an opinionated example configuration for JDTLS with most options enabled:

```jsonc
"lsp": {
  "jdtls": {
    "initialization_options": {
      "bundles": [],
      // The extension automatically sets this to the worktree root.
      // Override only if your Java project root differs from the opened folder:
      // "workspaceFolders": ["file:///path/to/your/java/project"],
      "settings": {
        "java": {
          "configuration": {
            "updateBuildConfiguration": "automatic",
            "runtimes": []
          },
          "saveActions": {
            "organizeImports": true
          },
          "compile": {
            "nullAnalysis": {
              "mode": "automatic"
            }
          },
          "references": {
            "includeAccessors": true,
            "includeDecompiledSources": true
          },
          "jdt": {
            "ls": {
              "protobufSupport": {
                "enabled": true
              },
              "groovySupport": {
                "enabled": true
              }
            }
          },
          "eclipse": {
            "downloadSources": true
          },
          "maven": {
            "downloadSources": true,
            "updateSnapshots": true
          },
          "autobuild": {
            "enabled": true
          },
          "maxConcurrentBuilds": 1,
          "inlayHints": {
            "parameterNames": {
              "enabled": "all"
            }
          },
          "signatureHelp": {
            "enabled": true,
            "description": {
              "enabled": true
            }
          },
          "format": {
            "enabled": true,
            "settings": {
              // The formatter config to use
              "url": "~/.config/jdtls/palantir_java_jdtls.xml"
            },
            "onType": {
              "enabled": true
            }
          },
          "contentProvider": {
            "preferred": null
          },
          "import": {
            "gradle": {
              "enabled": true,
              "wrapper": {
                "enabled": true
              }
            },
            "maven": {
              "enabled": true
            },
            "exclusions": [
              "**/node_modules/**",
              "**/.metadata/**",
              "**/archetype-resources/**",
              "**/META-INF/maven/**",
              "/**/test/**"
            ]
          },
          "completion": {
            "enabled": true,
            "favoriteStaticMembers": [
              "org.junit.Assert.*",
              "org.junit.Assume.*",
              "org.junit.jupiter.api.Assertions.*",
              "org.junit.jupiter.api.Assumptions.*",
              "org.junit.jupiter.api.DynamicContainer.*",
              "org.junit.jupiter.api.DynamicTest.*",
              "org.mockito.Mockito.*",
              "org.mockito.ArgumentMatchers.*"
            ],
            "importOrder": [
              "java",
              "javax",
              "com",
              "org"
            ],
            "postfix": {
              "enabled": true
            },
            "chain": {
              "enabled": true
            },
            "guessMethodArguments": "insertParameterNames",
            "overwrite": true
          },
          "errors": {
            "incompleteClasspath": {
              "severity": "warning"
            }
          },
          "implementationCodeLens": "all",
          "referencesCodeLens": {
            "enabled": true
          }
        }
      }
    }
  }
}
```

If you're working without a Gradle or Maven project, and the following error `The declared package "Example" does not match the expected package ""` pops up, consider adding these settings under

```
MyProject/
 ├── .zed/
 │   └── settings.json
 ```
 
```jsonc
"lsp": {
  "jdtls": {
    "initialization_options": {
      "project": {
        "sourcePaths": [
          ".",
          "src"
        ]
      },
    }
  }
}
```

If changes are not picked up, clean JDTLS' cache (from a java file run the task `Clear JDTLS cache`) and restart the language server.

## Architecture Note

The extension uses a native binary (`java-lsp-proxy`) that wraps the JDTLS process. This proxy enables the extension to communicate with JDTLS for features like debug class resolution and classpath queries. It is automatically downloaded from the [extension repository releases](https://github.com/zed-extensions/java/releases) and requires no user configuration.

## Developing Locally

If you want to contribute to this extension or test local changes, you can install it as a dev extension. Refer to the [Zed documentation on developing extensions](https://zed.dev/docs/extensions/developing-extensions) for full details.

### Prerequisites

- [Rust](https://rustup.rs/) toolchain
- The `wasm32-wasip1` target: `rustup target add wasm32-wasip1`
- [just](https://github.com/casey/just) command runner (optional but recommended)

### Installing as a Dev Extension

1. Clone the repository:
   ```sh
   git clone https://github.com/zed-extensions/java.git
   cd java
   ```

2. Make sure you are on the branch that contains the feature or fix you want to test:
   ```sh
   git branch --show-current
   # Switch if needed:
   git checkout <feature-branch>
   ```

3. In Zed, open the extensions panel (`zed: extensions` in the command palette), click the **Install Dev Extension** button, and select the cloned repository folder.

   Zed will build the WASM extension automatically and load it. After making changes to the extension source, use **Rebuild Dev Extension** from the command palette to pick them up.

### Using the `justfile`

The project includes a `justfile` with common development tasks:

| Recipe | Description |
|--------|-------------|
| `just proxy-build` | Build the proxy binary in debug mode |
| `just proxy-release` | Build the proxy binary in release mode |
| `just proxy-install` | Build release proxy and copy it to the extension workdir |
| `just ext-build` | Build the WASM extension in release mode |
| `just fmt` | Format all code (Rust + tree-sitter queries) |
| `just clippy` | Run clippy on both crates |
| `just lint` | Format and lint all code |
| `just all` | Lint, build extension, and install proxy |

### Updating the `java-lsp-proxy` Binary

The proxy is a separate native Rust binary (in the `proxy/` directory) that runs alongside the WASM extension. Because it's a native binary, it is **not** rebuilt when you use "Rebuild Dev Extension" — you need to build and install it manually.

> **Important:** When testing a manually built proxy, set `"check_updates": "never"` in your `lsp.jdtls.settings` to prevent the extension from downloading a release binary and overwriting your local build.

```sh
# Build the proxy in release mode and copy it to the extension workdir
just proxy-install
```

This compiles the proxy for your native target and copies it to the appropriate Zed extension working directory:
- **macOS**: `~/Library/Application Support/Zed/extensions/work/java/proxy-bin/`
- **Linux**: `~/.local/share/zed/extensions/work/java/proxy-bin/`
- **Windows**: `%LOCALAPPDATA%/Zed/extensions/work/java/proxy-bin/`

After installing the proxy, restart the language server in Zed for the changes to take effect.

If you prefer not to use `just`, you can build and copy manually:

```sh
cd proxy
cargo build --release --target $(rustc -vV | grep host | awk '{print $2}')
# Then copy the binary from target/<your-target>/release/java-lsp-proxy
# to the appropriate extension workdir shown above
```

### Remote Development (SSH)

When using [Zed's remote development](https://zed.dev/docs/remote-development) over SSH, extensions installed locally are automatically propagated to the remote server. The language server and the proxy binary run on the **remote host**, not your local machine.

For standard use, the proxy binary is auto-downloaded from GitHub releases for the remote server's platform — no action is needed.

However, if you're **testing local proxy changes** against a remote host, you need to get the binary onto the remote server yourself. The key thing to be aware of is that on remote hosts, extensions are stored under a **different path** than on your local machine — typically:

```
~/.local/share/zed/remote_extensions/work/java/proxy-bin/
```

> **Tip:** If you're unsure of the exact path, SSH into the remote and look for it:
> ```sh
> find ~/.local/share/zed -type d -name "proxy-bin" 2>/dev/null
> ```

#### Option A: Build on the remote directly

If you have Rust installed on the remote server, you can clone the repo there and build natively:

```sh
# On the remote host
git clone https://github.com/zed-extensions/java.git
cd java/proxy
cargo build --release

# Copy to the remote extensions workdir
mkdir -p ~/.local/share/zed/remote_extensions/work/java/proxy-bin
cp target/release/java-lsp-proxy ~/.local/share/zed/remote_extensions/work/java/proxy-bin/
```

#### Option B: Cross-compile locally and copy

If you prefer to build on your local machine:

1. Cross-compile the proxy for the remote target (typically Linux x86_64 or aarch64):
   ```sh
   cd proxy
   cargo build --release --target x86_64-unknown-linux-gnu
   ```
   > You may need to install the target first: `rustup target add x86_64-unknown-linux-gnu` and configure a linker in `.cargo/config.toml`.

2. Copy the binary to the remote server:
   ```sh
   scp target/x86_64-unknown-linux-gnu/release/java-lsp-proxy \
     user@remote:~/.local/share/zed/remote_extensions/work/java/proxy-bin/java-lsp-proxy
   ```

After either option, restart the language server in Zed for the changes to take effect.
