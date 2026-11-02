# Instructions

- Comments must begin with a capital and not end with a period.
- Separate logical steps within functions with blank lines.
- Keep egui layout closures short and extract panel bodies, item rendering, and request handling into focused, named helpers.
- Prefer early returns and descriptive local names over deeply nested branches and long compound expressions.
- Keep tests that need private access inline in the owning module; do not widen production visibility for tests. Keep other tests in one `tests.rs` per top-level source area, grouped by component and registered from that area's `mod.rs`.
