# Zed Java

This extension adds support for the Java language.

## Configuration Options

If [Lombok] support is enabled via [JDTLS] configuration option
(`settings.java.jdt.ls.lombokSupport.enabled`), this
extension will download and add [Lombok] as a javaagent to the JVM arguments for
[JDTLS].

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
                "severity": "warning",
              },
            },
            "configuration": {
              "updateBuildConfiguration": "interactive",
              "maven": {
                "userSettings": null,
              },
            },
            "trace": {
              "server": "verbose",
            },
            "import": {
              "gradle": {
                "enabled": true,
              },
              "maven": {
                "enabled": true,
              },
              "exclusions": [
                "**/node_modules/**",
                "**/.metadata/**",
                "**/archetype-resources/**",
                "**/META-INF/maven/**",
                "/**/test/**",
              ],
            },
            "jdt": {
              "ls": {
                "lombokSupport": {
                  "enabled": false, // Set this to true to enable lombok support
                },
              },
            },
            "referencesCodeLens": {
              "enabled": false,
            },
            "signatureHelp": {
              "enabled": false,
            },
            "implementationsCodeLens": {
              "enabled": false,
            },
            "format": {
              "enabled": true,
            },
            "saveActions": {
              "organizeImports": false,
            },
            "contentProvider": {
              "preferred": null,
            },
            "autobuild": {
              "enabled": false,
            },
            "completion": {
              "favoriteStaticMembers": [
                "org.junit.Assert.*",
                "org.junit.Assume.*",
                "org.junit.jupiter.api.Assertions.*",
                "org.junit.jupiter.api.Assumptions.*",
                "org.junit.jupiter.api.DynamicContainer.*",
                "org.junit.jupiter.api.DynamicTest.*",
              ],
              "importOrder": ["java", "javax", "com", "org"],
            },
          },
        },
      },
    },
  },
}
```

_Example taken from JDTLS's [configuration options wiki page]._

You can see all the options JDTLS accepts [here][configuration options wiki page].

[JDTLS]: https://github.com/eclipse-jdtls/eclipse.jdt.ls
[configuration options wiki page]: https://github.com/eclipse-jdtls/eclipse.jdt.ls/wiki/Running-the-JAVA-LS-server-from-the-command-line#initialize-request
[Lombok]: https://projectlombok.org
