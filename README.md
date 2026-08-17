# Yoko2D

A Rust reimplementation of [Seamly2D](https://github.com/FashionFreedom/Seamly2D),
an open-source pattern-drafting CAD application originally written in C++/Qt.

This is **Phase 0**: pure project scaffolding. There is no pattern-drawing,
geometry, or formula-evaluation logic yet — just the workspace layout,
placeholder types/functions proving the dependency graph is wired correctly,
and CI.

## Crate layout

```
Yoko2D/
├── Cargo.toml                 workspace root
├── rust-toolchain.toml        pinned stable toolchain
├── crates/
│   ├── core/                  data model + formula engine (lib)
│   ├── io/                    file format read/write (lib)
│   ├── render/                PatternData -> draw commands (lib)
│   ├── app/                   egui/eframe GUI application (bin: yoko2d-app)
│   └── cli/                   headless tool (bin: yoko2d-cli)
├── fixtures/                  sample pattern/measurement files (future test fixtures)
└── .github/workflows/ci.yml   build/test/lint CI
```

## Dependency direction

```
core   (no workspace-internal dependencies)
io     -> core
render -> core
app    -> core, io, render
cli    -> core, io
```

## Building

```sh
cargo build --workspace
cargo test --workspace
```
