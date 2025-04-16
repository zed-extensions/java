# Zed Java

This extension adds support for the Java language. Using [JDTLS] language server.

## Lombok

If [Lombok] support is enabled via [JDTLS] configuration option
(`settings.java.jdt.ls.lombokSupport.enabled`), this
extension will download and add [Lombok] as a java agent to the JVM arguments for
[JDTLS].

## Configuration Options

You can see all the options which [JDTLS] accepts [here](OPTIONS.md).
Example configuration:

```jsonc
{
  "lsp": {
    // ...
    "jdtls": {
      "initialization_options": {
        "bundles": [],
        "workspaceFolders": ["file:///path/to/project/folder"],
      },
      "settings": {
        "java": {
          // java.home deprecated use 'java.jdt.ls.java.home' instead.
          // Absolute path to JDK home folder used to launch the Java Language Server.
          "jdt": { "ls": { "java": { "home": "/path/to/jdk" } } },

          // Enable/disable the 'auto build'.
          "autobuild": {
            "enabled": false,
          },

          "completion": {
            // Specify how the arguments will be filled during completion.
            // Supported values are: "auto", "off", "insertParameterNames", "insertBestGuessedArguments"
            "guessMethodArguments": "off",
          },

          "inlayhints": {
            // Enable/disable inlay hints for parameter names.
            // Supported values are: "none", "literals", "all"
            "parameterNames": {
              "enabled": "all",
            },
          },
        },
      },
    },
    // ...
  },
}
```

[JDTLS]: https://github.com/eclipse-jdtls/eclipse.jdt.ls
[Lombok]: https://projectlombok.org
