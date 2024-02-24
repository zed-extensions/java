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
        ]* @context)
    "class" @context
    name: (_) @name) @item

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
        ]* @context)
    "record" @context
    name: (_) @name) @item

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
        ]* @context)
    "interface" @context
    name: (_) @name) @item

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
        ]* @context)
    name: (_) @name
    parameters: (formal_parameters
      "(" @context
      ")" @context)) @item

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
        ]* @context)
    declarator: (variable_declarator
        name: (_) @name)) @item
