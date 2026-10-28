# Interface translations

ODR embeds UTF-8 JSON catalogs in its Rust executable. No gettext compiler,
runtime catalog installation or additional Rust dependency is needed. `en.json`
is the source-message inventory; `fr`, `es`, `zh_CN`, `hi`, `ar` and `pt` contain
the corresponding translations. English is the fallback. Native GTK dialogs
still use the desktop's GTK translations for their own internal controls.

## Adding and translating messages

- Mark scalar GtkBuilder `label`, `title`, `tooltip-text`, `placeholder-text`,
  chooser labels and menu labels with `translatable="yes"`. Use
  `crate::i18n::builder()` to load the template. Translated text is XML-escaped.
- Use `tr("Choose a source")` for Rust text and
  `crate::trf!("Next scan: {source}", source = source.name)` for dynamic messages.
  Keep complete sentences together; placeholders may be reordered.
- Add the English key to every catalog. Preserve placeholder names exactly,
  literal shortcuts, engine names, units and significant newlines. Use count
  labels where appropriate so zero, one, two and larger counts read naturally.
- Do not translate file contents, paths, source names, CSS classes, widget IDs,
  command flags, JSON fields or database values. Dropdown models retain their
  raw values; a GTK factory translates only the displayed labels. Translate
  persisted scan states at display time with `i18n::state()`.

The locale is initialized once, before GUI worker threads. `ODR_LANGUAGE` wins
over the saved preference, which wins over the desktop locale. Regional variants
fall back to a supported base language. Changing the saved preference takes
effect on the next launch and never mutates a running engine's environment.
Subprocesses continue using `LC_ALL=C`; CLI output remains stable.

## Validation

```sh
cargo test --locked
cargo build --locked
sh scripts/check-i18n.sh --catalogs-only
sh scripts/check-i18n.sh
ODR_LANGUAGE=fr sh scripts/check-ui-advanced.sh
ODR_LANGUAGE=ar sh scripts/check-ui-advanced.sh
```

Catalog checks reject missing messages, empty translations and changed
placeholders. Unit tests cover locale fallback, preference persistence and
interpolation of filenames containing braces. GTK checks cover menus, dialogs,
RTL direction, text inputs, dropdown values and multi-engine selection in all
seven languages. Inspect production screenshots with disposable sources for
long labels and missing glyphs; Noto fonts provide Chinese and Hindi coverage.
