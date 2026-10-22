# Interface languages

Settings → Language offers English and ten complete owned-interface catalogs:
Spanish, German, French, Brazilian Portuguese, Italian, Russian, Simplified
Chinese, Japanese, Korean and Polish. The default follows the Windows display
language. Selection is saved locally and takes effect without restarting;
unsupported saved codes fall back to English. The menu uses native language names.

The desktop inventory contains **600 translatable strings and 58 intentional
technical invariants per catalog**. Coverage includes the five tools, navigation,
feature lists, settings, tooltips, scope/consent guidance and owned status templates.
All ten catalogs were directly authored and semantically reviewed, including
read-only limits, privileges/network access, cancellation effects and recovery
constraints. This is agent-performed review, not independent native-speaker signoff.

Original diagnostic reports and evidence, captured logs, OS/dependency errors,
commands/arguments, paths, vendor reviews/output, exported JSON and CLI help keep
their original wording. Translating presentation must never alter an executable
argument, immutable review, saved profile, receipt or diagnostic artifact. Unknown
or empty keys fall back to English. Numeric metadata comparison does not become a
build-success or compatibility guarantee in another language.

Run `python scripts/ui-translation-inventory.py`, then
`python scripts/translation-coverage.py` to regenerate the inventory and audit.
`LOCALIZATION-COVERAGE.json` records missing values, duplicate-key rejection,
placeholder occurrence integrity, invariant preservation and equal-source labels
requiring human review (e.g. German/Polish “System”). Coverage counts alone do not
certify linguistic quality or every possible OS/vendor message. Rust tests check
embedded catalog coverage and formatting fail-closed behavior. Isolated layout
subprocesses exercise all eleven language choices at preferred/minimum tool widths,
including expanded configuration sections; actual screen-reader use remains a
separate verification limit.

Catalogs are embedded. No translation service, model, account or download is needed.
Windows system CJK fonts are used when available; no font installation occurs.
Development-only translation-model probes were rejected; their generated drafts
and model weights are not part of the product.

Translation keys are English strings in `lang/<code>.json`. Preserve exact
placeholder occurrences, safety meaning, product names and technical identifiers.
Formatted UI values are inserted once without interpreting their contents as
translation placeholders. Local overrides may be placed in `lang` beside the
executable; a valid override replaces that language's catalog, so provide a complete
catalog to retain its embedded coverage. Files are limited to 1 MiB, 4096 entries,
bounded key/value lengths and at most 64 directory entries. No translation file
is executed. Custom catalogs are locally trusted presentation data.

Independent second-agent review on 2026-10-22 checked all 6,000 owned translations, corrected 80 wording issues, and preserved placeholders/technical identifiers. See LOCALIZATION-REVIEW.json for exact changes. This does not substitute for human native-speaker review.
