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
