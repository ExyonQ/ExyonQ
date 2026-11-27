# GeoIP test database

`GeoIP2-Country-Test.mmdb` is MaxMind's public test database, not production
GeoIP data. It is used only by unit/integration tests for the MMDB reader and
the request-path country policy.

- Source: <https://github.com/maxmind/MaxMind-DB/raw/main/test-data/GeoIP2-Country-Test.mmdb>
- License: Apache-2.0 OR MIT (MaxMind-DB)
- SHA-256: `fc12d6470b35b70bd3e4534dd433d3e262a6c73f7682f85c344e832d4190f02b`
- Size: 17,300 bytes
- Pinned upstream blob: `530ef08f867510e22054609c638c87754d113be4`

Known records used by the tests:

- `81.2.69.160` → `GB` (Europe)
- `89.160.20.128` → `SE` (Europe)
