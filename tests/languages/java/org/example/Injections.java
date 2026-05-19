package org.example;

// This is a line comment injection content.
/* This is a block comment injection content. */
/**
 * This is a Javadoc comment matching doxygen-like rules.
 */
public class Injections {
    public void test() {
        System.out.printf("Printf format string: %s %d\n", "hello", 123);
        String.format("Format string: %x", 255);
        String formatted = "Formatted string literal: %s".formatted("example");
    }
}
