(package_header
  "package" @context
  (identifier) @name) @item

(class_declaration
  (modifiers)? @context
  (type_identifier) @name) @item

(object_declaration
  "object" @context
  (type_identifier) @name) @item

(type_alias
  "typealias" @context
  (type_identifier) @name) @item

(enum_entry
  (simple_identifier) @name) @item

(function_declaration
  "fun" @context
  (simple_identifier) @name) @item

(property_declaration
  [
    "val"
    "var"
  ] @context
  (variable_declaration
    (simple_identifier) @name)) @item

(property_declaration
  [
    "val"
    "var"
  ] @context
  (multi_variable_declaration
    (variable_declaration
      (simple_identifier) @name) @item))

(companion_object
  "companion" @context
  "object" @context
  (type_identifier)? @name) @item

(secondary_constructor
  "constructor" @name) @item

(anonymous_initializer
  "init" @name) @item

; --- Gradle build-script DSL -------------------------------------------------
; The declarations above cover generic Kotlin, but a `build.gradle.kts` is mostly
; configuration blocks and assignments. Surface those so the outline reflects the
; build structure rather than just the rare top-level `val`/`fun`/`class`.
; Configuration blocks: `name { … }` — a call with a trailing lambda, e.g.
; `plugins { … }`, `dependencies { … }`, `repositories { … }`, `doLast { … }`.
; The lambda body is captured as `@item` so members nest underneath.
(call_expression
  (simple_identifier) @name
  (call_suffix
    (annotated_lambda
      (lambda_literal) @item)))

; Task containers with a name argument and a trailing lambda, e.g.
; `tasks.register("myTask") { … }`, `tasks.named<Test>("test") { … }`. The
; method (`register`/`named`) is the context and the string name is the label.
(call_expression
  (call_expression
    (navigation_expression
      (navigation_suffix
        (simple_identifier) @context))
    (call_suffix
      (value_arguments
        (value_argument
          (string_literal) @name))))
  (call_suffix
    (annotated_lambda
      (lambda_literal) @item)))

; Top-level property assignments, e.g. `group = "com.example"`, `version = "…"`.
(assignment
  (directly_assignable_expression
    (simple_identifier) @name)) @item
