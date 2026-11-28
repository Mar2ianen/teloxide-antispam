# Unicode 18.0 conformance data

These are official Unicode 18.0.0 files, downloaded from the Unicode UCD and
UTS #39 security data directories. Tests use them offline to validate ICU4X
normalization, case folding, general categories, script assignments, binary
properties, default ignorables, and the UTS #39 confusable skeleton.

| File | Source | SHA-256 |
| --- | --- | --- |
| `NormalizationTest.txt` | `https://www.unicode.org/Public/18.0.0/ucd/NormalizationTest.txt` | `25a50d816764b04abfb4a646d3eb2b2a803284c3873d9a06757b94fe4513dde3` |
| `CaseFolding.txt` | `https://www.unicode.org/Public/18.0.0/ucd/CaseFolding.txt` | `a004797658a457bec4dc11683e39f69249ea3b595b752dbea6721c4c9f587b0d` |
| `DerivedCoreProperties.txt` | `https://www.unicode.org/Public/18.0.0/ucd/DerivedCoreProperties.txt` | `09c928886a178fcafd93c29e4bd59073a058e5a100b716d425cb563ab50f68c9` |
| `PropList.txt` | `https://www.unicode.org/Public/18.0.0/ucd/PropList.txt` | `f438f532e8737bb8a2702126cdf9c4af5e357c58c7acf9d9eb2fc7c1a1d955d6` |
| `Scripts.txt` | `https://www.unicode.org/Public/18.0.0/ucd/Scripts.txt` | `0071fd81b6aeae25f6e8bce8efec3066a6476a91b49bdb2f52dc76e817862a6a` |
| `UnicodeData.txt` | `https://www.unicode.org/Public/18.0.0/ucd/UnicodeData.txt` | `0736451de439ae7baf1425136617da495e09ee5afbe6e394374db7009ea08950` |
| `confusables.txt` | `https://www.unicode.org/Public/18.0.0/security/confusables.txt` | `6ed3ee967c9dfdf6677d563c9985182fbc50a2efb7d6059cd57b2e2ce18f5b92` |

Unicode data files are provided under the Unicode Terms of Use and Unicode
Data Files and Software License: <https://www.unicode.org/terms_of_use.html>.

The exhaustive tests compare `Default_Ignorable_Code_Point` with
`DerivedCoreProperties.txt`, and `White_Space`, `Bidi_Control`, and
`Variation_Selector` with `PropList.txt`, over every Unicode scalar value.
