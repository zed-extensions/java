# JDTLS Configuration Parameters

This is the documentations for all options of JDTLS. All option descriptions are __stolen__ from [this](https://github.com/redhat-developer/vscode-java/blob/main/package.json) and [this](https://github.com/eclipse-jdtls/eclipse.jdt.ls/blob/main/org.eclipse.jdt.ls.core/src/org/eclipse/jdt/ls/core/internal/preferences/Preferences.java#L1099) files.

## Startup

### `java.home`

- **Description**: Specifies the folder path to the JDK (21 or more recent) used to launch the Java Language Server. On Windows, backslashes must be escaped, i.e. "java.home":"C:\\Program Files\\Java\\jdk-21.0_5"
- **Default**: null

### `java.jdt.ls.protobufSupport.enabled`

- **Description**: Specify whether to automatically add Protobuf output source directories to the classpath.
- **Note:** Only works for Gradle `com.google.protobuf` plugin `0.8.4` or higher.
- **Default**: true

### `java.jdt.ls.androidSupport.enabled`

- **Description**: [Experimental] Specify whether to enable Android project importing. When set to `auto`, the Android support will be enabled in Visual Studio Code - Insiders.
- **Note:** Only works for Android Gradle Plugin `3.2.0` or higher.
- **Options**:
  - auto
  - on
  - off
- **Default**: "auto"

### `java.jdt.ls.javac.enabled`

- **Description**: [Experimental] Specify whether to enable Javac-based compilation in the language server. Requires running this extension with Java 23
- **Options**:
  - on
  - off
- **Default**: "off"

## Project Import/Update

### `java.configuration.updateBuildConfiguration`

- **Description**: Specifies how modifications on build files update the Java classpath/configuration
- **Options**:
  - disabled
  - interactive
  - automatic
- **Default**: "interactive"

### `java.import.exclusions`

- **Description**: Configure glob patterns for excluding folders. Use `!` to negate patterns to allow subfolders imports. You have to include a parent directory. The order is important.
- **Default**:

   ```json
   [
     "**/node_modules/**",
     "**/.metadata/**",
     "**/archetype-resources/**",
     "**/META-INF/maven/**"
   ]
   ```

### `java.project.resourceFilters`

- **Description**: Excludes files and folders from being refreshed by the Java Language Server, which can improve the overall performance. For example, ["node_modules","\.git"] will exclude all files and folders named `node_modules` or `.git`. Pattern expressions must be compatible with `java.util.regex.Pattern`. Defaults to ["node_modules","\.git"].
- **Default**: ["node_modules", "\\.git"]

### `java.project.encoding`

- **Description**: Project encoding settings
- **Options**:
  - ignore: Ignore project encoding settings
  - warning: Show warning if a project has no explicit encoding set
  - setDefault: Set the default workspace encoding settings
- **Default**: "ignore"

## Unmanaged Folder

### `java.project.sourcePaths`

- **Description**: Relative paths to the workspace where stores the source files. `Only` effective in the `WORKSPACE` scope. The setting will `NOT` affect Maven or Gradle project.
- **Default**: []

### `java.project.outputPath`

- **Description**: A relative path to the workspace where stores the compiled output. `Only` effective in the `WORKSPACE` scope. The setting will `NOT` affect Maven or Gradle project.
- **Default**: ""

### `java.project.referencedLibraries`

- **Description**: Configure glob patterns for referencing local libraries to a Java project.
- **Default**: ["lib/**/*.jar"]

## Maven

### `java.import.maven.enabled`

- **Description**: Enable/disable the Maven importer.
- **Default**: true

### `java.import.maven.offline.enabled`

- **Description**: Enable/disable the Maven offline mode.
- **Default**: false

### `java.import.maven.disableTestClasspathFlag`

- **Description**: Enable/disable test classpath segregation. When enabled, this permits the usage of test resources within a Maven project as dependencies within the compile scope of other projects.
- **Default**: false

### `java.maven.downloadSources`

- **Description**: Enable/disable download of Maven source artifacts as part of importing Maven projects.
- **Default**: false

### `java.maven.updateSnapshots`

- **Description**: Force update of Snapshots/Releases.
- **Default**: false

### `java.configuration.maven.userSettings`

- **Description**: Path to Maven's user settings.xml
- **Default**: null

### `java.configuration.maven.globalSettings`

- **Description**: Path to Maven's global settings.xml
- **Default**: null

### `java.configuration.maven.notCoveredPluginExecutionSeverity`

- **Description**: Specifies severity if the plugin execution is not covered by Maven build lifecycle.
- **Options**:
  - ignore
  - warning
  - error
- **Default**: "warning"

### `java.configuration.maven.defaultMojoExecutionAction`

- **Description**: Specifies default mojo execution action when no associated metadata can be detected.
- **Options**:
  - ignore
  - warn
  - error
  - execute
- **Default**: "ignore"

### `java.configuration.maven.lifecycleMappings`

- **Description**: Path to Maven's lifecycle mappings xml
- **Default**: null

## Gradle

### `java.import.gradle.enabled`

- **Description**: Enable/disable the Gradle importer.
- **Default**: true

### `java.import.gradle.wrapper.enabled`

- **Description**: Use Gradle from the 'gradle-wrapper.properties' file.
- **Default**: true

### `java.import.gradle.version`

- **Description**: Use Gradle from the specific version if the Gradle wrapper is missing or disabled.
- **Default**: null

### `java.import.gradle.home`

- **Description**: Use Gradle from the specified local installation directory or GRADLE_HOME if the Gradle wrapper is missing or disabled and no 'java.import.gradle.version' is specified.
- **Default**: null

### `java.import.gradle.java.home`

- **Description**: The location to the JVM used to run the Gradle daemon.
- **Default**: null

### `java.import.gradle.user.home`

- **Description**: Setting for GRADLE_USER_HOME.
- **Default**: null

### `java.import.gradle.offline.enabled`

- **Description**: Enable/disable the Gradle offline mode.
- **Default**: false

### `java.import.gradle.arguments`

- **Description**: Arguments to pass to Gradle.
- **Default**: null

### `java.import.gradle.jvmArguments`

- **Description**: JVM arguments to pass to Gradle.
- **Default**: null

### `java.import.gradle.annotationProcessing.enabled`

- **Description**: Enable/disable the annotation processing on Gradle projects and delegate Annotation Processing to JDT APT. Only works for Gradle 5.2 or higher.
- **Default**: true

### `java.imports.gradle.wrapper.checksums`

- **Description**: Defines allowed/disallowed SHA-256 checksums of Gradle Wrappers
- **Default**: []

## Build

### `java.autobuild.enabled`

- **Description**: Enable/disable the 'auto build'
- **Default**: true

### `java.maxConcurrentBuilds`

- **Description**: Max simultaneous project builds
- **Default**: 1

### `java.settings.url`

- **Description**: Specifies the url or file path to the workspace Java settings. See [Setting Global Preferences](https://github.com/redhat-developer/vscode-java/wiki/Settings-Global-Preferences)
- **Default**: null

### `java.compile.nullAnalysis.nonnull`

- **Description**: Specify the Nonnull annotation types to be used for null analysis. If more than one annotation is specified, then the topmost annotation will be used first if it exists in project dependencies. This setting will be ignored if `java.compile.nullAnalysis.mode` is set to `disabled`
- **Default**: ["javax.annotation.Nonnull", "org.eclipse.jdt.annotation.NonNull", "org.springframework.lang.NonNull"]

### `java.compile.nullAnalysis.nullable`

- **Description**: Specify the Nullable annotation types to be used for null analysis. If more than one annotation is specified, then the topmost annotation will be used first if it exists in project dependencies. This setting will be ignored if `java.compile.nullAnalysis.mode` is set to `disabled`
- **Default**: ["javax.annotation.Nullable", "org.eclipse.jdt.annotation.Nullable", "org.springframework.lang.Nullable"]

### `java.compile.nullAnalysis.nonnullbydefault`

- **Description**: Specify the NonNullByDefault annotation types to be used for null analysis. If more than one annotation is specified, then the topmost annotation will be used first if it exists in project dependencies. This setting will be ignored if `java.compile.nullAnalysis.mode` is set to `disabled`
- **Default**: ["javax.annotation.ParametersAreNonnullByDefault", "org.eclipse.jdt.annotation.NonNullByDefault", "org.springframework.lang.NonNullApi"]

### `java.compile.nullAnalysis.mode`

- **Description**: Specify how to enable the annotation-based null analysis.
- **Options**:
  - disabled
  - interactive
  - automatic
- **Default**: "interactive"

### `java.errors.incompleteClasspath.severity`

- **Description**: Specifies the severity of the message when the classpath is incomplete for a Java file
- **Options**:
  - ignore
  - info
  - warning
  - error
- **Default**: "warning"

## Installed JDKs

### `java.configuration.runtimes`

- **Description**: Map Java Execution Environments to local JDKs.
- **Default**: []

## Formatting

### `java.format.enabled`

- **Description**: Enable/disable default Java formatter
- **Default**: true

### `java.format.insertSpaces`

- **Description**: Replace tabs with spaces
- **Default**: true

### `java.format.tabSize`

- **Description**: Tab size
- **Default**: true

### `java.format.settings.url`

- **Description**: Specifies the url or file path to the [Eclipse formatter xml settings](https://github.com/redhat-developer/vscode-java/wiki/Formatter-settings).
- **Default**: null

### `java.format.settings.profile`

- **Description**: Optional formatter profile name from the Eclipse formatter settings.
- **Default**: null

### `java.format.comments.enabled`

- **Description**: Includes the comments during code formatting.
- **Default**: true

### `java.format.onType.enabled`

- **Description**: Enable/disable automatic block formatting when typing `;`, `<enter>` or `}`
- **Default**: true

## Code Completion

### `java.completion.enabled`

- **Description**: Enable/disable code completion support
- **Default**: true

### `java.completion.overwrite`

- **Description**: Enable/disable overwriting code by completion. When set to true, code completion overwrites the current text. When set to false, code is simply added instead.
- **Default**: true

### `java.completion.engine`

- **Description**: [Experimental] Select code completion engine
- **Options**:
  - ecj
  - dom
- **Default**: "ecj"

### `java.completion.postfix.enabled`

- **Description**: Enable/disable postfix completion support. `#editor.snippetSuggestions#` can be used to customize how postfix snippets are sorted.
- **Default**: true

### `java.completion.chain.enabled`

- **Description**: Enable/disable chain completion support. Chain completions are only available when completions are invoked by the completions shortcut
- **Default**: false

### `java.completion.favoriteStaticMembers`

- **Description**: Defines a list of static members or types with static members. Content assist will propose those static members even if the import is missing.
- **Default**:

   ```json
   [
     "org.junit.Assert.*",
     "org.junit.Assume.*",
     "org.junit.jupiter.api.Assertions.*",
     "org.junit.jupiter.api.Assumptions.*",
     "org.junit.jupiter.api.DynamicContainer.*",
     "org.junit.jupiter.api.DynamicTest.*",
     "org.mockito.Mockito.*",
     "org.mockito.ArgumentMatchers.*",
     "org.mockito.Answers.*"
   ]
   ```

### `java.completion.filteredTypes`

- **Description**: Defines the type filters. All types whose fully qualified name matches the selected filter strings will be ignored in content assist or quick fix proposals and when organizing imports. For example 'java.awt.*' will hide all types from the awt packages.
- **Default**:

   ```json
   [
     "java.awt.*",
     "com.sun.*",
     "sun.*",
     "jdk.*",
     "org.graalvm.*",
     "io.micrometer.shaded.*"
   ]
   ```

### `java.completion.guessMethodArguments`

- **Description**: Specify how the arguments will be filled during completion.
- **Options**:
  - auto: Use 'off' only when using Visual Studio Code - Insiders, other platform will defaults to 'insertBestGuessedArguments'.
  - off: Method arguments will not be inserted during completion.
  - insertParameterNames: The parameter names will be inserted during completion.
  - insertBestGuessedArguments: The best guessed arguments will be inserted during completion according to the code context.
- **Default**: "auto"

### `java.completion.matchCase`

- **Description**: Specify whether to match case for code completion.
- **Options**:
  - firstLetter: Match case for the first letter when doing completion.
  - off: Do not match case when doing completion.
- **Default**: "firstLetter"

### `java.completion.importOrder`

- **Description**: Defines the sorting order of import statements. A package or type name prefix (e.g. 'org.eclipse') is a valid entry. An import is always added to the most specific group. As a result, the empty string (e.g. '') can be used to group all other imports. Static imports are prefixed with a '#'
- **Default**: ["#", "java", "javax", "org", "com", ""]

### `java.completion.lazyResolveTextEdit.enabled`

- **Description**: [Experimental] Enable/disable lazily resolving text edits for code completion.
- **Default**: true

### `java.completion.maxResults`

- **Description**: Maximum number of completion results (not including snippets).
`0` (the default value) disables the limit, all results are returned. In case of performance problems, consider setting a sensible limit.
- **Default**: 0

### `java.signatureHelp.enabled`

- **Description**: Enable/disable the signature help.
- **Default**: true

### `java.signatureHelp.description.enabled`

- **Description**: Enable/disable to show the description in signature help.
- **Default**: false

### `java.completion.collapseCompletionItems`

- **Description**: Enable/disable the collapse of overloaded methods in completion items. Overrides `#java.completion.guessMethodArguments#`.
- **Default**: false

## Code Generation

### `java.templates.fileHeader`

- **Description**: Specifies the file header comment for new Java file. Supports configuring multi-line comments with an array of strings, and using ${variable} to reference the [predefined variables](command:_java.templateVariables).
- **Default**: []

### `java.templates.typeComment`

- **Description**: Specifies the type comment for new Java type. Supports configuring multi-line comments with an array of strings, and using ${variable} to reference the [predefined variables](command:_java.templateVariables).
- **Default**: []

### `java.codeGeneration.insertionLocation`

- **Description**: Specifies the insertion location of the code generated by source actions.
- **Options**:
  - afterCursor: Insert the generated code after the member where the cursor is located.
  - beforeCursor: Insert the generated code before the member where the cursor is located.
  - lastMember: Insert the generated code as the last member of the target type.
- **Default**: "afterCursor"

### `java.codeGeneration.addFinalForNewDeclaration`

- **Description**: Whether to generate the 'final' modifer for code actions that create new declarations.
- **Options**:
  - none: Do not generate final modifier.
  - fields: Generate 'final' modifier only for new field declarations.
  - variables: Generate 'final' modifier only for new variable declarations.
  - all: Generate 'final' modifier for all new declarations.
- **Default**: "none"

### `java.codeGeneration.hashCodeEquals.useJava7Objects`

- **Description**: Use Objects.hash and Objects.equals when generating the hashCode and equals methods. This setting only applies to Java 7 and higher.
- **Default**: false

### `java.codeGeneration.hashCodeEquals.useInstanceof`

- **Description**: Use 'instanceof' to compare types when generating the hashCode and equals methods.
- **Default**: false

### `java.codeGeneration.useBlocks`

- **Description**: Use blocks in 'if' statements when generating the methods.
- **Default**: false

### `java.codeGeneration.generateComments`

- **Description**: Generate method comments when generating the methods.
- **Default**: false

### `java.codeGeneration.toString.template`

- **Description**: The template for generating the toString method.
- **Default**: "${object.className} [${member.name()}=${member.value}, ${otherMembers}]"

### `java.codeGeneration.toString.codeStyle`

- **Description**: The code style for generating the toString method.
- **Options**:
  - STRING_CONCATENATION: String concatenation
  - STRING_BUILDER: StringBuilder/StringBuffer
  - STRING_BUILDER_CHAINED: StringBuilder/StringBuffer - chained call
  - STRING_FORMAT: String.format/MessageFormat
- **Default**: "STRING_CONCATENATION"

### `java.codeGeneration.toString.skipNullValues`

- **Description**: Skip null values when generating the toString method.
- **Default**: false

### `java.codeGeneration.toString.listArrayContents`

- **Description**: List contents of arrays instead of using native toString().
- **Default**: true

### `java.codeGeneration.toString.limitElements`

- **Description**: Limit number of items in arrays/collections/maps to list, if 0 then list all.
- **Default**: 0

### `java.edit.smartSemicolonDetection.enabled`

- **Description**: Defines the `smart semicolon` detection. Defaults to `false`.
- **Default**: false

## Code Action

### `java.rename.enabled`

- **Description**: Enable/disable the symbol renaming.
- **Default**:

### `java.cleanup.actions`

- **Description**: The list of clean ups to be run on the current document when it's saved or when the cleanup command is issued. Clean ups can automatically fix code style or programming mistakes. Click [HERE](command:_java.learnMoreAboutCleanUps) to learn more about what each clean up does.
- **Default**: ["renameFileToType"]

### `java.cleanup.actionsOnSave`

- **Description**: None
- **Default**: []

### `java.saveActions.cleanup`

- **Description**: Enable/disable cleanup actions on save.
- **Default**: true

### `java.saveActions.organizeImports`

- **Description**: Enable/disable auto organize imports on save action
- **Default**: false

### `java.sources.organizeImports.starThreshold`

- **Description**: Specifies the number of imports added before a star-import declaration is used.
- **Default**: 99

### `java.sources.organizeImports.staticStarThreshold`

- **Description**: Specifies the number of static imports added before a star-import declaration is used.
- **Default**: 99

### `java.quickfix.showAt`

- **Description**: Show quickfixes at the problem or line level.
- **Options**:
  - line
  - problem
- **Default**: "line"

### `java.codeAction.sortMembers.avoidVolatileChanges`

- **Description**: Reordering of fields, enum constants, and initializers can result in semantic and runtime changes due to different initialization and persistence order. This setting prevents this from occurring.
- **Default**: true

### `java.memberSortOrder`

- **Description**: Preference that defines how member elements are ordered by code actions. Each entry must be in the list, no duplication. List order defines the sort order.
- **Options**:
  - **T**: Types
  - **C**: Constructors
  - **I**: Initializers
  - **M**: Methods
  - **F**: Fields
  - **SI**: Static Initializers
  - **SM**: Static Methods
  - **SF**: Static Fields
- **Default**: null

### `java.refactoring.extract.interface.replace`

- **Description**: Specify whether to replace all the occurrences of the subtype with the new extracted interface.
- **Default**: true

## Code Navigation

### `java.referencesCodeLens.enabled`

- **Description**: Enable/disable the references code lens.
- **Default**: false

### `java.implementationCodeLens`

- **Description**: Enable/disable the implementations code lens for the provided categories.
- **Options**:
  - none: Disable the implementations code lens
  - types: Enable the implementations code lens only for types
  - methods: Enable the implementations code lens only for methods
  - all: Enable the implementations code lens for types and methods
- **Default**: "none"

### `java.references.includeAccessors`

- **Description**: Include getter, setter and builder/constructor when finding references.
- **Default**: true

### `java.references.includeDecompiledSources`

- **Description**: Include the decompiled sources when finding references.
- **Default**: true

### `java.symbols.includeSourceMethodDeclarations`

- **Description**: Include method declarations from source files in symbol search.
- **Default**: false

### `java.inlayHints.parameterNames.enabled`

- **Description**: Enable/disable inlay hints for parameter names:

   ```java
   Integer.valueOf(/* s: */ '123', /* radix: */ 10)
   ```

- **Options**:
  - none: Disable parameter name hints
  - literals: Enable parameter name hints only for literal arguments
  - all: Enable parameter name hints for literal and non-literal arguments
- **Default**: "literals"

### `java.inlayHints.parameterNames.exclusions`

- **Description**: The patterns for the methods that will be disabled to show the inlay hints. Supported pattern examples:
  - `java.lang.Math.*` - All the methods from java.lang.Math.
  - `*.Arrays.asList` - Methods named as 'asList' in the types named as 'Arrays'.
  - `*.println(*)` - Methods named as 'println'.
  - `(from, to)` - Methods with two parameters named as 'from' and 'to'.
  - `(arg*)` - Methods with one parameter whose name starts with 'arg'.
- **Default**: []

### `java.search.scope`

- **Description**: Specifies the scope which must be used for search operation like
  - Find Reference
  - Call Hierarchy
  - Workspace Symbols
- **Options**:
  - all: Search on all classpath entries including reference libraries and projects.
  - main: All classpath entries excluding test classpath entries.
- **Default**: "all"

## Others

### `java.telemetry.enabled`

- **Description**: Enable/disable the telemetry
- **Default**: false

### `java.eclipse.downloadSources`

- **Description**: Enable/disable download of Maven source artifacts for Eclipse projects.
- **Default**: false

### `java.contentProvider.preferred`

- **Description**: Preferred content provider (a 3rd party decompiler id, usually)
- **Default**: null

### `java.foldingRange.enabled`

- **Description**: Enable/disable smart folding range support. If disabled, it will use the default indentation-based folding range provided by VS Code.
- **Default**: true

### `java.executeCommand.enabled`

- **Description**: Enable/disable executeCommand.
- **Default**: true

### `java.selectionRange.enabled`

- **Description**: Enable/disable Smart Selection support for Java. Disabling this option will not affect the VS Code built-in word-based and bracket-based smart selection.
- **Default**: true

### `java.edit.validateAllOpenBuffersOnChanges`

- **Description**: Specifies whether to recheck all open Java files for diagnostics when editing a Java file.
- **Default**: false

### `java.diagnostic.filter`

- **Description**: Specifies a list of file patterns for which matching documents should not have their diagnostics reported (eg. '**/Foo.java').
- **Default**: []
