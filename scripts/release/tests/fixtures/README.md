# P14SIGN Phase 3 test fixtures

This directory may hold **non-secret** public fixtures only.

Private keys, Cosign passphrases, and ephemeral signing material must **never**
be stored here. The Phase 3 harness generates TEST_ONLY keys under
`.exyonq-local/tmp/` and shreds them on exit.
