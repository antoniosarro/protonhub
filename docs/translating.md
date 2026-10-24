# Translating

The GUI is translated with GNU gettext; the CLI stays English, since its
output is for scripts too. Messages are English text with `{name}`
placeholders, which a translation may move but must keep:

```
msgid "{game} now runs with {tool}"
msgstr "{game} ora usa {tool}"
```

Plurals use the language's own forms (`msgstr[0]`, `msgstr[1]`, ...), with
`{n}` the count.

## Adding a language

In the dev shell:

```sh
just pot                                                   # po/protonhub.pot from the code
msginit --locale=de_DE.UTF-8 -i po/protonhub.pot -o po/de.po
# translate po/de.po
just locale                                                # compile, checking every entry
LANGUAGE=de just gui                                       # look at it
```

`just gui` uses the compiled catalogs in `target/locale`; the Nix package
and the Flatpak install them to `share/locale`. Keep an existing
translation current with `just pot`, which merges new messages into every
`po/*.po`; `msgfmt --statistics po/de.po` says what's left.

## Languages

| Language | File |
|---|---|
| Italian | `po/it.po` |
