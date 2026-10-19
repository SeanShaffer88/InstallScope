# Translating InstallScope

InstallScope uses the English text itself as the translation key. Each language is a
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
Create a `lang` folder next to `InstallScope.exe` and put your file in it, named after the language code (for example `lang/pl.json` or `lang/ko.json`). Restart InstallScope and pick the language under **View → Language** or **Settings**. A file with the same code as a built-in language replaces it, which is handy for improving an existing translation.

## Adding it permanently
1. Add `lang/<code>.json`.
2. Add it to the `BUILTIN` list in `src/i18n.rs`.
3. Run `cargo test --lib` — it checks that every placeholder survived.
4. Open a pull request.

The built-in translations were written for the first release and would benefit from review by native speakers.
