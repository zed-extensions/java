; Run the main function
(
    (package_declaration
        (scoped_identifier) @java_package_name
    )
    (class_declaration
        name: (identifier) @java_class_name
        body: (class_body
            (method_declaration
                name: (identifier) @run
                (#eq? @run "main")
            )
        )
    ) @_
    (#set! tag java-main)
)

; Run the main function
(
    (package_declaration
        (scoped_identifier) @java_package_name
    )
    (class_declaration
        name: (identifier) @java_class_name @run
        body: (class_body
            (method_declaration
                name: (identifier) @method_name
                (#eq? @method_name "main")
            )
        )
    ) @_
    (#set! tag java-main)
)

; Run the test function
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

; Run the test function
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
