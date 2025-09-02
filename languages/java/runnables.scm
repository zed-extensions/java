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
