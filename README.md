# Zed Java

This extension adds support for the Java language.

## Configuration

### Settings

You can optionally configure the version of [JDTLS] (the language server) to
download or the class path that [JDTLS] uses in your Zed settings.

If [lombok] support is enabled via [JDTLS] initialization option
(`initialization_options.settings.java.jdt.ls.lombokSupport.enabled`), this
extension will add [lombok] as a javaagent to the JVM arguments for [JDTLS].
You can also configure the version of [lombok] to use via setting the version
at `settings.lombok_version`.

Below is a configuration example for this extension:

```jsonc
{
  "lsp": {
    "jdtls": {
      "settings": {
        "classpath": "/path/to/classes.jar:/path/to/more/classes/",
        "jdtls_version": "1.40.0", // This is the default value
        "lombok_version": "1.18.34" // Defaults to latest version if not set
      }
    }
  }
}
```

### Initialization Options

There are also many more options you can pass directly to the language server,
for example:

```jsonc
{
  "lsp": {
    "jdtls": {
      "initialization_options": {
        "bundles": [],
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
            "trace": {
              "server": "verbose"
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
            "lombokSupport": {
              "enabled": false // Set this to true to enable lombok support
            },
            "referencesCodeLens": {
              "enabled": false
            },
            "signatureHelp": {
              "enabled": false
            },
            "implementationsCodeLens": {
              "enabled": false
            },
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
```

*Example taken from JDTLS's [initialization options wiki page].*

You can see all the options JDTLS accepts [here][initialization options wiki
page].

[JDTLS]: https://github.com/eclipse-jdtls/eclipse.jdt.ls
[initialization options wiki page]: https://github.com/eclipse-jdtls/eclipse.jdt.ls/wiki/Running-the-JAVA-LS-server-from-the-command-line#initialize-request
[lombok]: https://projectlombok.org
