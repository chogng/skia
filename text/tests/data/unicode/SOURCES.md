# Unicode conformance sources

The full conformance files are downloaded into `target/` instead of being
stored in Git. `scripts/fetch_unicode_conformance.sh` pins every URL and
SHA-256 digest.

The versions intentionally follow the Unicode data advertised by the exact
text dependencies in `text/Cargo.toml`:

- `unicode-segmentation 1.13.3`: Unicode 17.0.0,
  `GraphemeBreakTest-17.0.0.txt`;
- `unicode-linebreak 0.1.5`: Unicode 15.0.0,
  `LineBreakTest-15.0.0.txt`;
- `unicode-bidi 0.3.18`: Unicode 16.0.0,
  `BidiCharacterTest-16.0.0.txt`.

The files are provided by Unicode, Inc. under the Unicode License v3. See
<https://www.unicode.org/license.txt>.

Run the complete suite from the repository root:

```sh
scripts/fetch_unicode_conformance.sh
SKIA_UNICODE_CONFORMANCE_DIR=target/unicode-conformance \
  cargo test -p skia-text --test unicode_conformance -- --ignored
```

All three files are strict conformance gates. The line-break adapter applies
the test file's documented regex-number tailoring and covers the Unicode 15
LB30 East-Asian-width exception plus LB30b potential-emoji rule that are not
represented by `unicode-linebreak 0.1.5`'s pair table.
