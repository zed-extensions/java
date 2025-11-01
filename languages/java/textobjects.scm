; methods
(method_declaration) @function.around
(method_declaration
  body: (block
    "{" (_)* @function.inside "}"
  ))

; constructors
(constructor_declaration) @function.around
(constructor_declaration
  body: (constructor_body
    "{" (_)* @function.inside "}"
  ))

; lambdas
(lambda_expression) @function.around
(lambda_expression
  body: (block
    "{" (_)* @function.inside "}"
  ))
(lambda_expression
  body: (_) @function.inside)


; classes
(class_declaration) @class.around
(class_declaration
  body: (class_body
    "{" (_)* @class.inside "}"
  ))

; interfaces
(interface_declaration) @class.around
(interface_declaration
  body: (interface_body
    "{" (_)* @class.inside "}"
  ))

; enums
(enum_declaration) @class.around
(enum_declaration
  body: (enum_body
    "{"
    _* @class.inside
    "}"
  ))

; records
(record_declaration) @class.around
(record_declaration
  body: (class_body
    "{" (_)* @class.inside "}"
  ))

; annotations
(annotation_type_declaration) @class.around
(annotation_type_declaration
  (annotation_type_body
    "{" (_)* @class.inside "}"
  ))


; comments
((line_comment)+) @comment.around
(block_comment) @comment.around
