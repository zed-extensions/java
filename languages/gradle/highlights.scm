[
  "!in"
  "!instanceof"
  "as"
  "assert"
  "case"
  "catch"
  "class"
  "def"
  "default"
  "else"
  "extends"
  "finally"
  "for"
  "if"
  "import"
  "in"
  "instanceof"
  "package"
  "pipeline"
  "return"
  "switch"
  "try"
  "while"
  (break)
  (continue)
] @keyword

[
  "true"
  "false"
] @boolean

(null) @constant

"this" @variable.builtin

[
  "int"
  "char"
  "short"
  "long"
  "boolean"
  "float"
  "double"
  "void"
] @type.builtin

[
  "final"
  "private"
  "protected"
  "public"
  "static"
  "synchronized"
] @type.qualifier

(comment) @comment

(shebang) @comment

(string) @string

(string
  (escape_sequence) @operator)

(string
  (interpolation
    "$" @operator))

"(" @punctuation.bracket

")" @punctuation.bracket

"[" @punctuation.bracket

"]" @punctuation.bracket

"{" @punctuation.bracket

"}" @punctuation.bracket

":" @punctuation.delimiter

"," @punctuation.delimiter

"." @punctuation.delimiter

(number_literal) @number

(identifier) @variable

((identifier) @variable.parameter
  (#is? @variable.parameter "local.parameter"))

; An identifier on the left of an assignment (e.g. `group = '…'`, `version = '…'`
; in a build script) is a property reference, not a parameter. Re-capture it as
; `@variable` after the `@variable.parameter` rule above so it wins last-match.
(assignment
  .
  (identifier) @variable)

; Members of a dotted access (e.g. `tasks` in `tasks.named(…)`, the receiver
; segments of `project.ext.foo`) are object/property references, not parameters.
; Re-capture as `@variable` after the parameter rule; the `function_call` rules
; below still override the trailing method segment to `@function`.
(dotted_identifier
  (identifier) @variable)

((identifier) @constant
  (#match? @constant "^[A-Z][A-Z_]+"))

[
  "%"
  "*"
  "/"
  "+"
  "-"
  "<<"
  ">>"
  ">>>"
  ".."
  "..<"
  "<..<"
  "<.."
  "<"
  "<="
  ">"
  ">="
  "=="
  "!="
  "<=>"
  "==="
  "!=="
  "=~"
  "==~"
  "&"
  "^"
  "|"
  "&&"
  "||"
  "?:"
  "+"
  "*"
  ".&"
  ".@"
  "?."
  "*."
  "*"
  "*:"
  "++"
  "--"
  "!"
] @operator

(string
  "/" @string)

(ternary_op
  ([
    "?"
    ":"
  ]) @operator)

(map
  (map_item
    key: (identifier) @variable.parameter))

(parameter
  type: (identifier) @type
  name: (identifier) @variable.parameter)

(generic_param
  name: (identifier) @variable.parameter)

(declaration
  type: (identifier) @type)

(function_definition
  type: (identifier) @type)

(function_declaration
  type: (identifier) @type)

(class_definition
  name: (identifier) @type)

(class_definition
  superclass: (identifier) @type)

(generic_param
  superclass: (identifier) @type)

(type_with_generics
  (identifier) @type)

(type_with_generics
  (generics
    (identifier) @type))

(generics
  [
    "<"
    ">"
  ] @punctuation.bracket)

(generic_parameters
  [
    "<"
    ">"
  ] @punctuation.bracket)

; TODO: Class literals with PascalCase
(declaration
  "=" @operator)

(assignment
  "=" @operator)

(function_call
  function: (identifier) @function)

(function_call
  function: (dotted_identifier
    (identifier) @function .))

(function_call
  (argument_list
    (map_item
      key: (identifier) @variable.parameter)))

(juxt_function_call
  function: (identifier) @function)

(juxt_function_call
  function: (dotted_identifier
    (identifier) @function .))

(juxt_function_call
  (argument_list
    (map_item
      key: (identifier) @variable.parameter)))

(function_definition
  function: (identifier) @function)

(function_declaration
  function: (identifier) @function)

(annotation) @function.macro

(annotation
  (identifier) @function.macro)

"@interface" @function.macro

"pipeline" @keyword

(groovy_doc) @comment.documentation

(groovy_doc
  [
    (groovy_doc_param)
    (groovy_doc_throws)
    (groovy_doc_tag)
  ] @string.special)

(groovy_doc
  (groovy_doc_param
    (identifier) @variable.parameter))

(groovy_doc
  (groovy_doc_throws
    (identifier) @type))
