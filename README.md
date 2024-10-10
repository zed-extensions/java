# Zed Java

This extension adds support for the [Java](https://github.com/zed-extensions/java) language.

## Configuration

### `java_home`

You can optionally configure the Java home that JDTLS (the language server) uses
in your Zed settings like so:

```json
{
  "lsp": {
    "jdtls": {
      "settings": {
        "java_home": "/path/to/jdk/"
      }
    }
  }
}
```

### `classpath`

You can also configure the class PATH that JDTLS will use like so:

```json
{
  "lsp": {
    "jdtls": {
      "settings": {
        "classpath": "/path/to/classes.jar:/path/to/more/classes/"
      }
    }
  }
}
```
