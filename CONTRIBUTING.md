# Contributing to InstallScope

Thanks for helping! Bug reports, translations, ideas and code are all welcome.

## Reporting a bug
Open an issue with: Windows version, InstallScope version (Help → About), what you did, what happened, and — if relevant — the exported install log (Install logs → Export → JSON). Remove anything private from it first.

## Code
- Rust stable, `cargo fmt` before committing, `cargo clippy` clean where possible.
- The engine lives in `src/` (library crate `installscope_core`); the interface lives in `src/gui/`; the command line in `src/cli.rs`.
- Anything that deletes or changes things **must** go through `revert::execute_plan` or a `BackupWriter`, and must respect `util::is_protected_dir` / `util::is_protected_key`.
- Add a unit test for engine changes. `MemRegistry` lets you test registry logic on any OS: `cargo test --lib`.
- Every user-visible string goes through `t("...")` or `tf("...", &[...])` so it can be translated.

## Translations
See [docs/TRANSLATING.md](docs/TRANSLATING.md).

## License
By contributing you agree that your contributions are licensed under the MIT License.
