# rust-derive-selflog

Rust derive that generates methods that log the fields of a struct, once or every interval while a closure runs.

- Every release bumps `version` in `Cargo.toml` and gets a matching `v<version>` tag.
- Dependencies are only `proc-macro2`, `quote` and `syn`. Generated `log` calls `::tracing` with the `tracing` feature and `println!` without it.
- Every function, struct and enum has a one-line `///` doc comment, generated methods included.
- `just lint` passes before every commit.
