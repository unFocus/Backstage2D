# Scripting

Goal: a small language designed for timeline-driven interactive content.
Marker scripts, event handlers, and object behaviors should be easy to write,
typed enough to give good editor tooling, and hot-reloadable.

## Options

| Option | Pros | Cons |
|---|---|---|
| **Custom language + bytecode VM** | Designed for timeline and display list concepts. We control semantics, debugging, and serialization of VM state. | Largest effort: parser, type checker, VM, LSP. |
| **Rhai** | Pure Rust, easy to embed, sandboxed. | Dynamic typing. Slower. Tooling is limited. |
| **Lua (mlua / Luau)** | Proven, fast, well known. Luau adds gradual types. | C dependency. Semantics don't match ours. |
| **WASM guest modules** | Any source language, sandboxed, fast. | Heavy iteration loop. Awkward for small marker scripts. |

## Proposed path
1. Define the **host API** first (display list, timeline control, events,
   tweening) as a Rust trait surface that doesn't depend on any language.
2. Prototype on Rhai or Luau to test the API cheaply.
3. Build the custom language against the same host API once its shape is
   settled.

## Custom language sketch (strawman)
- Expression-oriented, statically typed with inference, and no nulls
  (`Option`).
- First-class `on` handlers: `on update(dt) { ... }`, `on marker("land") { ... }`, `on click(btn) { ... }`.
- Coroutines/`await` over time: `await seconds(0.5)`, `await tween(...)`, `await marker("land")`.
- Compiles to a register-based bytecode. VM state is serializable, so saving
  and rewinding work.
