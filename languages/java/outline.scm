(class_declaration
    (modifiers
        [
            "private"
            "public"
            "protected"
            "abstract"
            "sealed"
            "non-sealed"
            "final"
            "strictfp"
            "static"
        ]* @context)?
    "class" @context
    name: (_) @name
    body: (_) @item)

(record_declaration
    (modifiers
        [
            "private"
            "public"
            "protected"
            "abstract"
            "sealed"
            "non-sealed"
            "final"
            "strictfp"
            "static"
        ]* @context)?
    "record" @context
    name: (_) @name
    body: (_) @item)

(interface_declaration
    (modifiers
        [
            "private"
            "public"
            "protected"
            "abstract"
            "sealed"
            "non-sealed"
            "strictfp"
            "static"
        ]* @context)?
    "interface" @context
    name: (_) @name
    body: (_) @item)

(enum_declaration
    (modifiers
        [
            "private"
            "public"
            "protected"
            "static"
            "final"
            "strictfp"
        ]* @context)?
    "enum" @context
    name: (_) @name
    body: (_) @item)

(annotation_type_declaration
    (modifiers
        [
            "private"
            "public"
            "protected"
            "abstract"
            "static"
            "strictfp"
        ]* @context)?
    "@interface" @context
    name: (_) @name
    body: (_) @item)

(enum_constant
    name: (identifier) @name) @item

(method_declaration
    (modifiers
        [
            "private"
            "public"
            "protected"
            "abstract"
            "static"
            "final"
            "native"
            "strictfp"
            "synchronized"
        ]* @context)?
    type: (_) @context
    name: (_) @name
    parameters: (formal_parameters
      "(" @context
      (formal_parameter
        type: (_) @context)?
      ("," @context
        (formal_parameter
          type: (_) @context)?)*
      ")" @context)
    body: (_) @item)

(method_declaration
    (modifiers
        [
            "private"
            "public"
            "protected"
            "abstract"
            "static"
            "final"
            "native"
            "strictfp"
            "synchronized"
        ]* @context)?
    type: (_) @context
    name: (_) @name
    parameters: (formal_parameters
      "(" @context
      (formal_parameter
        type: (_) @context)?
      ("," @context
        (formal_parameter
          type: (_) @context)?)*
      ")" @context)
    ";"  @item)

(constructor_declaration
    (modifiers
        [
            "private"
            "public"
            "protected"
            "static"
            "final"
        ]* @context)?
    name: (_) @name
    parameters: (formal_parameters
      "(" @context
      (formal_parameter
        type: (_) @context)?
      ("," @context
        (formal_parameter
          type: (_) @context)?)*
      ")" @context)
    body: (_) @item)

(compact_constructor_declaration
    (modifiers
        [
            "private"
            "public"
            "protected"
        ]* @context)?
    name: (_) @name
    body: (_) @item)

(annotation_type_element_declaration
    (modifiers
        [
            "private"
            "public"
            "protected"
            "abstract"
            "static"
        ]* @context)?
    type: (_) @context
    name: (_) @name
    "(" @context
    ")" @context) @item

(field_declaration
    (modifiers
        [
            "private"
            "public"
            "protected"
            "static"
            "final"
            "transient"
            "volatile"
        ]* @context)?
    type: (_) @context
    declarator: (variable_declarator
        name: (_) @name)) @item

(constant_declaration
    (modifiers
        [
            "public"
            "static"
            "final"
        ]* @context)?
    type: (_) @context
    declarator: (variable_declarator
        name: (_) @name)) @item

(static_initializer
    "static" @context
    (block) @item)

(record_declaration
    parameters: (formal_parameters
        (formal_parameter
            type: (_) @context
            name: (_) @name) @item))
