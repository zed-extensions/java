package org.example;

public class Outline {
    public static final String CONSTANT_FIELD = "value";
    private String instanceField;

    static {
        // static initializer block
        System.out.println("Static Init");
    }

    public Outline() {
        this.instanceField = "default";
    }

    public Outline(String value) {
        this.instanceField = value;
    }

    public String getInstanceField() {
        return instanceField;
    }

    public static void staticMethod() {
        // static method
    }

    private static class NestedClass {
        private int nestedField;
    }
}

interface ExampleInterface {
    int INTERFACE_CONSTANT = 42;

    void abstractMethod();

    default void defaultMethod() {
        // default method
    }
}

record ExampleRecord(String first, int second) {
    public ExampleRecord {
        // compact constructor
    }
}

enum ExampleEnum {
    FIRST_CONSTANT,
    SECOND_CONSTANT;
}

@interface ExampleAnnotation {
    String value() default "default_val";
}
