; Run the main function
(
    (package_declaration
        (scoped_identifier) @java_package_name
    )?
    (class_declaration
        (modifiers) @class-modifier
        (#match? @class-modifier "public")
        name: (identifier) @java_class_name
        body: (class_body
            (method_declaration
                (modifiers) @modifier
                name: (identifier) @run
                (#eq? @run "main")
                (#match? @modifier "public")
                (#match? @modifier "static")
            )
        )
    ) @_
    (#set! tag java-main)
)

; Run the main class
(
    (package_declaration
        (scoped_identifier) @java_package_name
    )?
    (class_declaration
        (modifiers) @class-modifier
        (#match? @class-modifier "public")
        name: (identifier) @java_class_name @run
        body: (class_body
            (method_declaration
                (modifiers) @modifier
                name: (identifier) @method_name
                (#eq? @method_name "main")
                (#match? @modifier "public")
                (#match? @modifier "static")
            )
        )
    ) @_
    (#set! tag java-main)
)

; Run test function
(
    (package_declaration
        (scoped_identifier) @java_package_name
    )
    (class_declaration
        name: (identifier) @java_class_name
        body: (class_body
            (method_declaration
                (modifiers
                    (marker_annotation
                        name: (identifier) @annotation_name
                    )
                )
                name: (identifier) @run @java_method_name
                (#eq? @annotation_name "Test")
            )
        )
    ) @_
    (#set! tag java-test-method)
)

; Run test class
(
    (package_declaration
        (scoped_identifier) @java_package_name
    )
    (class_declaration
        name: (identifier) @java_class_name @run
        body: (class_body
            (method_declaration
                (modifiers
                    (marker_annotation
                        name: (identifier) @annotation_name
                    )
                )
                (#eq? @annotation_name "Test")
            )
        )
    ) @_
    (#set! tag java-test-class)
)
