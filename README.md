# Zed Java

This extension adds support for the [Java](https://github.com/zed-extensions/java) language.

## Configuration

You can optionally configure the Java home that JDTLS (the language server) uses
and/or the class path that it uses in your Zed settings like so:

```json
{
  "lsp": {
    "jdtls": {
      "settings": {
        "java_home": "/path/to/jdk/",
        "classpath": "/path/to/classes.jar:/path/to/more/classes/"
      }
    }
  }
}
```
