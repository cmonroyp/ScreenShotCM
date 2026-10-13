# Third-party licences

Every dependency is free and permissively licensed, with no fee. Check the
licence before adding anything, and record it here.

## Inside the executable

| Crate | Version | Licence |
|---|---|---|
| windows, windows-core, windows-collections, windows-future, windows-link, windows-numerics, windows-result, windows-strings, windows-threading | 0.62.2 family | MIT OR Apache-2.0 |

LLVM runtime pieces linked in by the toolchain (compiler-rt, libunwind) are
Apache-2.0 WITH LLVM-exception, which waives attribution in binary form. The
C runtime is the Universal CRT that ships with Windows 10 and 11.

## Build time only (not shipped)

| Component | Licence |
|---|---|
| windows-implement, windows-interface (proc macros) | MIT OR Apache-2.0 |
| syn, quote, proc-macro2 | MIT OR Apache-2.0 |
| unicode-ident | (MIT OR Apache-2.0) AND Unicode-3.0 |
| Rust toolchain (gnullvm) | MIT OR Apache-2.0 |
| llvm-mingw | ISC (scripts); LLVM: Apache-2.0 WITH LLVM-exception; mingw-w64: ZPL / public domain |
| WiX Toolset 5.0.2 (installer) | MS-RL |
