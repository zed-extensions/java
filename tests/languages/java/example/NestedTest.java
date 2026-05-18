package example;

import org.junit.jupiter.api.Nested;
import org.junit.jupiter.api.Test;

public class NestedTest {
    @Nested
    class Inner {
        @Test
        void testMethod() {}
    }
}
