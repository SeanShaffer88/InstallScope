# Translating SetupScope

SetupScope uses the English text itself as the translation key. Each language is a
flat JSON file in [`lang/`](../lang): the key is the English string and the
value is the translation.

```json
{
  "_language": "Deutsch",
  "Uninstall": "Deinstallieren",
  "{n} files": "{n} Dateien"
}
```

- `_language` is the language's own name, shown in the language menu.
- Keep placeholders like `{n}`, `{name}`, `{size}`, `{path}`, `{error}`, `{done}` and `{total}` exactly as they are (you may move them).
- Missing or empty entries fall back to English, so partial translations are fine.

## Trying a translation without rebuilding
Create a `lang` folder next to `SetupScope.exe` and put your file in it, named after the language code (for example `lang/pl.json` or `lang/ko.json`). Restart SetupScope and pick the language under **Settings → Language**. A file with the same code as a built-in language replaces it, which is handy for improving an existing translation.

## Adding it permanently
1. Add `lang/<code>.json`.
2. Add it to the `BUILTIN` list in `src/i18n.rs`.
3. Run `python scripts/translation-coverage.py` and `cargo test --lib` to verify coverage, technical invariants and exact placeholder occurrences.
4. Open a pull request.

The ten owned-interface catalogs are complete against `lang/ui-strings.json`; independent native-speaker review remains welcome. Original evidence, vendor reviews, commands and CLI help retain their original wording. See [coverage and limits](LOCALIZATION.md).
