# Instructions

- Comments must begin with a capital and not end with a period.
- Separate logical steps within functions with blank lines.
- Use attached opening braces in C and follow the repository's `uncrustify.cfg` configuration.
- Keep C signatures and calls on one line when they fit within 88 columns. Wrap longer lists vertically with one parameter or argument per line, one extra indentation level, and the closing parenthesis on its own line. Keep empty argument lists as `()`.
- Wrap multiline C control headers with the opening parenthesis after the keyword, the contents indented one level, and the closing `) {` on its own line. Put each clause of a wrapped `for` header on its own line.
- Keep egui layout closures short and extract panel bodies, item rendering, and request handling into focused, named helpers.
- Prefer early returns and descriptive local names over deeply nested branches and long compound expressions.
- Keep tests that need private access inline in the owning module; do not widen production visibility for tests. Keep other tests in one `tests.rs` per top-level source area, grouped by component and registered from that area's `mod.rs`.
