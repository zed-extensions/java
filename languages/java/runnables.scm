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

; Run test function (marker annotation, e.g. @Test)
(
    (package_declaration
        (scoped_identifier) @java_package_name
    )
    (class_declaration
        name: (identifier) @java_class_name
        body: (class_body
            (method_declaration
                (modifiers
                    [(marker_annotation
                        name: (identifier) @annotation_name
                    )
                    (annotation
                        name: (identifier) @annotation_name
                    )]
                )
                name: (identifier) @run @java_method_name
                (#match? @annotation_name "^(Test|ParameterizedTest|RepeatedTest)$")
            )
        )
    ) @_
    (#set! tag java-test-method)
)

; Run nested test function
(
    (package_declaration
        (scoped_identifier) @java_package_name
    )
    (class_declaration
        name: (identifier) @java_outer_class_name
        body: (class_body
            (class_declaration
                (modifiers
                    (marker_annotation
                        name: (identifier) @nested_annotation
                    )
                )
                name: (identifier) @java_class_name
                body: (class_body
                    (method_declaration
                        (modifiers
                            [(marker_annotation
                                name: (identifier) @annotation_name
                                )
                            (annotation
                                name: (identifier) @annotation_name
                                )]
                            )
                        name: (identifier) @run @java_method_name
                        (#match? @annotation_name "^(Test|ParameterizedTest|RepeatedTest)$")
                    )
                )
                (#eq? @nested_annotation "Nested")
            ) @_
        )
    )
    (#set! tag java-test-method-nested)
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
                    [(marker_annotation
                        name: (identifier) @annotation_name
                    )
                    (annotation
                        name: (identifier) @annotation_name
                    )]
                )
                (#match? @annotation_name "^(Test|ParameterizedTest|RepeatedTest)$")
            )
        )
    ) @_
    (#set! tag java-test-class)
)

; Run nested test class
(
    (package_declaration
        (scoped_identifier) @java_package_name
    )
    (class_declaration
        name: (identifier) @java_outer_class_name
        body: (class_body
            (class_declaration
                (modifiers
                    (marker_annotation
                        name: (identifier) @nested_annotation
                    )
                )
                name: (identifier) @run @java_class_name
                body: (class_body
                    (method_declaration
                        (modifiers
                            [(marker_annotation
                                name: (identifier) @annotation_name
                                )
                            (annotation
                                name: (identifier) @annotation_name
                                )]
                            )
                        (#match? @annotation_name "^(Test|ParameterizedTest|RepeatedTest)$")
                    )
                )
                (#eq? @nested_annotation "Nested")
            ) @_
        )
    )
    (#set! tag java-test-class-nested)
)
