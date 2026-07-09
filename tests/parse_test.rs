use tree_sitter::Parser;

#[test]
fn parse_simplified_main() {
    let source = r#"public class HelloWorld {
    main(String[] args) {
        IO.println("Hello, World!");
    }
}"#;
    let mut parser = Parser::new();
    let language = tree_sitter_java::LANGUAGE.into();
    parser.set_language(&language).unwrap();
    let tree = parser.parse(source, None).unwrap();
    let root = tree.root_node();

    println!("has_error: {}", root.has_error());
    println!(
        "root kind: {:?} children: {}",
        root.kind(),
        root.child_count()
    );
    dump_tree(source, root, 0);

    // The class body should have children
    // If main is an ERROR node, this won't have a method_declaration
    assert!(
        !root.has_error(),
        "Parser reported error for simplified main()"
    );
}

fn dump_tree(source: &str, node: tree_sitter::Node, indent: usize) {
    let prefix = "  ".repeat(indent);
    let kind = node.kind();
    let start = node.start_position();
    let end = node.end_position();
    if node.child_count() == 0 {
        let text = node.utf8_text(source.as_bytes()).unwrap_or("");
        println!(
            "{}{} [{},{} - {},{}] {:?}",
            prefix, kind, start.row, start.column, end.row, end.column, text
        );
    } else {
        println!(
            "{}{} [{},{} - {},{}] ({} children)",
            prefix,
            kind,
            start.row,
            start.column,
            end.row,
            end.column,
            node.child_count()
        );
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i as u32) {
                dump_tree(source, child, indent + 1);
            }
        }
    }
}
