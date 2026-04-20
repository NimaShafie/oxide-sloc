// Package doc comment.
package main

import "fmt"

// Greet returns a greeting.
func Greet(name string) string {
    return fmt.Sprintf("Hello, %s", name) /* inline block */
}
