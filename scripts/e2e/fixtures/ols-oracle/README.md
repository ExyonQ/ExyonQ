# OpenLiteSpeed oracle fixture

Pinned external oracle for the OL-03 comparison:

```text
image = litespeedtech/openlitespeed@sha256:91ca324386be3fe8cd91591112dbdd8e41af48cccbe4a48e3e8673a2921c8504
version = OLS 1.9.4
source commit = 031b2c8
listener = *:8088
workers = 1
```

`httpd_config.conf` is the oracle configuration used for the recorded OL-03
comparison. It sets `perClientConnLimit` to:

```text
softLimit = 1
hardLimit = 2
gracePeriod = 2
banPeriod = 3
outBandwidth = 32768
inBandwidth = 32768
```

`big256.bin` is generated at runtime as 262,144 bytes; it is not committed.
`scripts/e2e/ols-oracle-ci.sh` starts this image by digest and
`scripts/e2e/ol03-ols-compare.sh` rejects a container whose `Config.Image` does
not carry the pinned digest.
