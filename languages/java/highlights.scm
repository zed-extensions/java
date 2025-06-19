; CREDITS @maxbrunsfeld (maxbrunsfeld@gmail.com)
; Variables
(identifier) @variable

; Methods
(method_declaration
  name: (identifier) @function)

(method_invocation
  name: (identifier) @function)

(super) @function

; Parameters
(formal_parameter
  name: (identifier) @variable)

(catch_formal_parameter
  name: (identifier) @variable)

(spread_parameter
  (variable_declarator
    name: (identifier) @variable)) ; int... foo

; Lambda parameter
(inferred_parameters
  (identifier) @variable) ; (x,y) -> ...

(lambda_expression
  parameters: (identifier) @variable) ; x -> ...

; Operators
[
  "+"
  ":"
  "++"
  "-"
  "--"
  "&"
  "&&"
  "|"
  "||"
  "!"
  "!="
  "=="
  "*"
  "/"
  "%"
  "<"
  "<="
  ">"
  ">="
  "="
  "-="
  "+="
  "*="
  "/="
  "%="
  "->"
  "^"
  "^="
  "&="
  "|="
  "~"
  ">>"
  ">>>"
  "<<"
  "::"
] @operator

; Types
(interface_declaration
  name: (identifier) @type)

(annotation_type_declaration
  name: (identifier) @attribute)

(class_declaration
  name: (identifier) @type)

(record_declaration
  name: (identifier) @type)

(enum_declaration
  name: (identifier) @enum)

(enum_constant
  name: (identifier) @constant)

(constructor_declaration
  name: (identifier) @constructor)

(type_identifier) @type

((type_identifier) @type
  (#eq? @type "var"))

(object_creation_expression
  type: (type_identifier) @constructor)

((method_invocation
  object: (identifier) @type)
  (#match? @type "^[A-Z]"))

((method_reference
  .
  (identifier) @type)
  (#match? @type "^[A-Z]"))

((field_access
  object: (identifier) @type)
  (#match? @type "^[A-Z]"))

(scoped_identifier
  (identifier) @type
  (#match? @type "^[A-Z]"))

; Fields
(field_declaration
  declarator:
    (variable_declarator
      name: (identifier) @property))

(field_access
  field: (identifier) @property)

[
  (boolean_type)
  (integral_type)
  (floating_point_type)
  (void_type)
] @type

; Variables
((identifier) @constant
  (#match? @constant "^[A-Z_$][A-Z\\d_$]*$"))

(this) @variable

; Annotations
(annotation
  "@" @punctuation.special
  name: (identifier) @attribute)

(marker_annotation
  "@" @punctuation.special
  name: (identifier) @attribute)

; Literals
(string_literal) @string

(escape_sequence) @string.escape

(character_literal) @string

[
  (hex_integer_literal)
  (decimal_integer_literal)
  (octal_integer_literal)
  (binary_integer_literal)
  (decimal_floating_point_literal)
  (hex_floating_point_literal)
] @number

[
  (true)
  (false)
] @boolean

(null_literal) @constant.builtin

; Keywords
[
  "assert"
  "class"
  "record"
  "default"
  "enum"
  "extends"
  "implements"
  "instanceof"
  "interface"
  "@interface"
  "permits"
  "to"
  "with"
  "new"
] @keyword

[
  "abstract"
  "final"
  "native"
  "non-sealed"
  "open"
  "private"
  "protected"
  "public"
  "sealed"
  "static"
  "strictfp"
  "synchronized"
  "transitive"
] @keyword

[
  "transient"
  "volatile"
] @keyword

[
  "return"
  "yield"
] @keyword

; Conditionals
[
  "if"
  "else"
  "switch"
  "case"
  "when"
] @keyword

(ternary_expression
  [
    "?"
    ":"
  ] @operator)

; Loops
[
  "for"
  "while"
  "do"
  "continue"
  "break"
] @keyword

; Includes
[
  "exports"
  "import"
  "module"
  "opens"
  "package"
  "provides"
  "requires"
  "uses"
] @keyword

; Punctuation
[
  ";"
  "."
  "..."
  ","
] @punctuation.delimiter

[
  "{"
  "}"
] @punctuation.bracket

[
  "["
  "]"
] @punctuation.bracket

[
  "("
  ")"
] @punctuation.bracket

(type_arguments
  [
    "<"
    ">"
  ] @punctuation.bracket)

(type_parameters
  [
    "<"
    ">"
  ] @punctuation.bracket)

(string_interpolation
  [
    "\\{"
    "}"
  ] @punctuation.special) @embedded

; Exceptions
[
  "throw"
  "throws"
  "finally"
  "try"
  "catch"
] @keyword

; Labels
(labeled_statement
  (identifier) @label)

; Comments
[
  (line_comment)
  (block_comment)
] @comment

((block_comment) @comment.doc
  (#match? @comment.doc "^\\/\\*\\*"))
