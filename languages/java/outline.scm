(class_declaration
    (modifiers)? @context
    "class" @context
    name: (_) @name) @item

(method_declaration
    (modifiers) @context
    name: (_) @name
    parameters: (formal_parameters
      "(" @context
      ")" @context)) @item

(field_declaration
    (modifiers)*? @context
    declarator: (variable_declarator
        name: (_) @name)) @item
