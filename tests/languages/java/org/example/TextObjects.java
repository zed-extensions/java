package org.example;

// This is a block comment
// with multiple lines for testing
// comment.around
public class TextObjects {
    /*
     * This is a block comment in the class
     */
    private String name;

    public TextObjects(String name) {
        this.name = name;
    }

    public String getName() {
        return name;
    }

    public void runLambda() {
        Runnable r1 = () -> {
            System.out.println("Block lambda body");
        };

        Runnable r2 = () -> System.out.println("Expression lambda body");
    }
}

interface TestInterface {
    void process();

    default void log() {
        System.out.println("Interface log");
    }
}

enum TestEnum {
    A, B, C;

    public void print() {
        System.out.println(this.name());
    }
}

record TestRecord(int value) {
    public TestRecord {
        if (value < 0) {
            throw new IllegalArgumentException();
        }
    }
}

@interface TestAnnotation {
    String value();
}
