# Java Extension for Zed

This extension adds support for the Java language to [Zed](https://zed.dev). It is using the [Eclipse JDT Language Server](https://projects.eclipse.org/projects/eclipse.jdt.ls) (JDTLS for short) to provide completions, code-actions and diagnostics.

## Quick Start

Install the extension via Zeds extension manager. It should work out of the box for most people. However, there are some things to know:

- It is generally recommended to open projects with the Zed-project root at the Java project root folder (where you would commonly have your `pom.xml` or `build.gradle` file).

- By default the extension will download and run the latest official version of JDTLS for you, but this requires Java version 21 to be available on your system via either the `$JAVA_HOME` environment variable or as a `java(.exe)` executable on your `$PATH`. If your project requires a lower Java version in the environment, you can specify a different JDK to use for running JDTLS via the `java_home` configuration option.

- You can provide a **custom launch script for JDTLS**, by adding an executable named `jdtls` (or `jdtls.bat` on Windows) to your `$PATH` environment variable. If this is present, the extension will skip downloading and launching a managed instance and use the one from the environment.

- To support [Lombok](https://projectlombok.org/), the lombok-jar must be downloaded and registered as a Java-Agent when launching JDTLS. By default the extension automatically takes care of that, but in case you don't want that you can set the `lombok_support` configuration-option to `false`.

- The option to let the extension automatically download a version of OpenJDK can be enabled by setting `jdk_auto_download` to `true`. When enabled, the extension will only download a JDK if no valid java_home is provided or if the specified one does not meet the minimum version requirement. User-provided JDKs **always** take precedence.

Here is a common `settings.json` including the above mentioned configurations:

```jsonc
"lsp": {
 "jdtls": {
    "settings": {
      "java_home": "/path/to/your/JDK21+",
      "lombok_support": true,
      "jdk_auto_download": false,

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
      "java_debug_jar": "/path/to/your/com.microsoft.java.debug.plugin.jar"
    }
  }
}
```

## Project Symbol Search

The extension supports project-wide symbol search with syntax-highlighted results. This feature is powered by JDTLS and can be accessed via Zed's symbol search.

JDTLS uses **CamelCase fuzzy matching** for symbol queries. For example, searching for `EmpMe` would match `EmptyMedia`. The pattern works like `Emp*Me*`, matching the capital letters of CamelCase names.

## Debugger

Debug support is enabled via our [Fork of Java Debug](https://github.com/zed-industries/java-debug), which the extension will automatically download and start for you. Please refer to the [Zed Documentation](https://zed.dev/docs/debugger#getting-started) for general information about how debugging works in Zed.

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

## Launch Scripts (aka Tasks) in Windows

This extension provides tasks for running your application and tests from within Zed via little play buttons next to tests/entry points. However, due to current limitiations of Zed's extension interface, we can not provide scripts that will work across Maven and Gradle on both Windows and Unix-compatible systems, so out of the box the launch scripts only work on Mac and Linux.

There is a fairly straightforward fix that you can apply to make it work on Windows by supplying your own task scripts. Please see [this Issue](https://github.com/zed-extensions/java/issues/94) for information on how to do that and read the [Tasks section in Zeds documentation](https://zed.dev/docs/tasks) for more information.

## Advanced Configuration/JDTLS initialization Options
JDTLS provides many configuration options that can be passed via the `initialize` LSP-request. The extension will pass the JSON-object from `lsp.jdtls.settings.initialization_options` in your settings on to JDTLS. Please refer to the [JDTLS Configuration Wiki Page](https://github.com/eclipse-jdtls/eclipse.jdt.ls/wiki/Running-the-JAVA-LS-server-from-the-command-line#initialize-request) for the available options and values. Below is an example `settings.json` that would pass on the example configuration from the above wiki page to JDTLS:

```jsonc
{
  "lsp": {
    "jdtls": {
      "settings": {
          // this will be sent to JDTLS as initializationOptions:
        "initialization_options": {
          "bundles": [],
          // use this if your zed project root folder is not the same as the java project root:
          "workspaceFolders": ["file:///home/snjeza/Project"],
          "settings": {
            "java": {
              "home": "/usr/local/jdk-9.0.1",
              "errors": {
                "incompleteClasspath": {
                  "severity": "warning"
                }
              },
              "configuration": {
                "updateBuildConfiguration": "interactive",
                "maven": {
                  "userSettings": null
                }
              },
              "import": {
                "gradle": {
                  "enabled": true
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
              "referencesCodeLens": {
                "enabled": false
              },
              "signatureHelp": {
                "enabled": false
              },
              "implementationCodeLens": "all",
              "format": {
                "enabled": true
              },
              "saveActions": {
                "organizeImports": false
              },
              "contentProvider": {
                "preferred": null
              },
              "autobuild": {
                "enabled": false
              },
              "completion": {
                "favoriteStaticMembers": [
                  "org.junit.Assert.*",
                  "org.junit.Assume.*",
                  "org.junit.jupiter.api.Assertions.*",
                  "org.junit.jupiter.api.Assumptions.*",
                  "org.junit.jupiter.api.DynamicContainer.*",
                  "org.junit.jupiter.api.DynamicTest.*"
                ],
                "importOrder": ["java", "javax", "com", "org"]
              }
            }
          }
        }
      }
    }
  }
}
```
