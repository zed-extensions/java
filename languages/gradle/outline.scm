; Outline for Gradle build scripts (Groovy DSL).
;
; Build scripts are structured around configuration closures
; (`dependencies { … }`, `repositories { … }`, `android { … }`, `task foo { … }`),
; property assignments (`group = "…"`), `def`/typed declarations, and — in
; buildSrc / build logic — function and class definitions. We surface those so
; the symbol/breadcrumb navigation works for `.gradle` files.
;
; Following the Java `outline.scm` convention the closure/body is captured as
; `@item` so its members nest underneath it in the outline tree. We deliberately
; do not list every bare method call (e.g. `mavenCentral()`, `println …`) to keep
; the outline structural rather than one-row-per-statement.

; Configuration closures: `name { … }` — a call whose argument is a closure.
(juxt_function_call
  function: (identifier) @name
  (argument_list
    (closure) @item))

(juxt_function_call
  function: (dotted_identifier
    (identifier) @name .)
  (argument_list
    (closure) @item))

(function_call
  function: (identifier) @name
  (argument_list
    (closure) @item))

; Property assignments, e.g. `group = "com.example"`, `version = "1.0"`.
(assignment
  .
  (identifier) @name) @item

(assignment
  .
  (dotted_identifier) @name) @item

; `def`/typed declarations, e.g. `def libs = …`, `String x = …`.
(declaration
  name: (identifier) @name) @item

; Function and method definitions in build logic.
(function_definition
  function: (identifier) @name
  body: (closure) @item)

(function_declaration
  function: (identifier) @name) @item

; Class definitions (buildSrc / inline helper classes).
(class_definition
  name: (identifier) @name
  body: (closure) @item)
