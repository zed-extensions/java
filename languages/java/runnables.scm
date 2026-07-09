; Run main methods — all valid JVM entry-point variants
; Traditional: public static void main(String[]/String...)
; Modern (Java 21+): static/instance, with/without args, all inside a class
; Note: Without a return type, tree-sitter parses it as constructor_declaration
(program
  (package_declaration
    [
      (identifier)
      (scoped_identifier)
    ] @java_package_name)?
  (class_declaration
    name: (identifier) @java_class_name
    body: (class_body
      [
        (method_declaration
          name: (identifier) @run)
        (constructor_declaration
          name: (identifier) @run)
      ]
      (#eq? @run "main"))) @_
  (#set! tag java-main))

; Run main class — any class containing a main method
(program
  (package_declaration
    [
      (identifier)
      (scoped_identifier)
    ] @java_package_name)?
  (class_declaration
    name: (identifier) @java_class_name @run
    body: (class_body
      [
        (method_declaration
          name: (identifier) @method_name)
        (constructor_declaration
          name: (identifier) @method_name)
      ]
      (#eq? @method_name "main"))) @_
  (#set! tag java-main))

; Run top-level main method — implicitly declared class (Java 21+)
(program
  [
    (method_declaration
      name: (identifier) @run)
    (constructor_declaration
      name: (identifier) @run)
  ]
  (#eq? @run "main")
  (#set! tag java-main)) @_

; Run test function (marker annotation, e.g. @Test)
(program
  (package_declaration
    [
      (identifier)
      (scoped_identifier)
    ] @java_package_name)?
  (class_declaration
    name: (identifier) @java_class_name
    body: (class_body
      (method_declaration
        (modifiers
          [
            (marker_annotation
              name: (identifier) @annotation_name)
            (annotation
              name: (identifier) @annotation_name)
          ])
        name: (identifier) @run @java_method_name
        (#match? @annotation_name "Test$")))) @_
  (#set! tag java-test-method))

; Run nested test function
(program
  (package_declaration
    [
      (identifier)
      (scoped_identifier)
    ] @java_package_name)?
  (class_declaration
    name: (identifier) @java_outer_class_name
    body: (class_body
      (class_declaration
        (modifiers
          (marker_annotation
            name: (identifier) @nested_annotation))
        name: (identifier) @java_class_name
        body: (class_body
          (method_declaration
            (modifiers
              [
                (marker_annotation
                  name: (identifier) @annotation_name)
                (annotation
                  name: (identifier) @annotation_name)
              ])
            name: (identifier) @run @java_method_name
            (#match? @annotation_name "Test$")))
        (#eq? @nested_annotation "Nested")) @_))
  (#set! tag java-test-method-nested))

; Run test class
(program
  (package_declaration
    [
      (identifier)
      (scoped_identifier)
    ] @java_package_name)?
  (class_declaration
    name: (identifier) @java_class_name @run
    body: (class_body
      (method_declaration
        (modifiers
          [
            (marker_annotation
              name: (identifier) @annotation_name)
            (annotation
              name: (identifier) @annotation_name)
          ])
        (#match? @annotation_name "Test$")))) @_
  (#set! tag java-test-class))

; Run nested test class
(program
  (package_declaration
    [
      (identifier)
      (scoped_identifier)
    ] @java_package_name)?
  (class_declaration
    name: (identifier) @java_outer_class_name
    body: (class_body
      (class_declaration
        (modifiers
          (marker_annotation
            name: (identifier) @nested_annotation))
        name: (identifier) @run @java_class_name
        body: (class_body
          (method_declaration
            (modifiers
              [
                (marker_annotation
                  name: (identifier) @annotation_name)
                (annotation
                  name: (identifier) @annotation_name)
              ])
            (#match? @annotation_name "Test$")))
        (#eq? @nested_annotation "Nested")) @_))
  (#set! tag java-test-class-nested))
