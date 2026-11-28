# Public release notes

Since 0.9.3 the body of every GitHub Release is `docs/release/notes/v<version>.md`.

Keep it short: **80 lines / 8 KB maximum**, enforced by
`scripts/gates/release-notes-gate.sh` in `release.yml`. From 0.9.4 on, a tag
whose note is missing or oversized fails before the Release is created; earlier
tags keep the body they were published with.

The full record — verification tables, seal citations, publication details,
checklists — stays in `docs/release/v<version>.md` and
`docs/security/audit-v<version>.md`. Those are repository documents, not release
pages: 0.9.3 shipped its 612-line, 51 KB audit record as the GitHub body and read
like a log instead of a release announcement.

## Shape

One summary sentence (what changed and whether to upgrade) → `## Changes`
bullets → `## Install` → `## Verify` → links to the full record. Aim for the
10–30 line range nginx and OpenLiteSpeed use.

## Template

```markdown
**Security patch / feature release.** One or two sentences: what changed, upgrade advice.

## Changes

- **Feature:** one line, public issue number if there is one.
- **Fix:** one line.

## Install

Assets (`tar.gz`, `.deb`, `.rpm`), `docker pull` lines, tag policy: `:VERSION`,
never `:latest`.

## Verify

`shasum -a 256 -c SHA256SUMS.txt`; tag signature; state signing that is not
implemented instead of implying it.

Full notes: [docs/release/vX.Y.Z.md](url) · [CHANGELOG](url) · [audit seal](url)
```

## Checklist

- [ ] `docs/release/notes/v<version>.md` exists and passes
      `bash scripts/gates/release-notes-gate.sh --version <version>`.
- [ ] Every claim matches `CHANGELOG.md` and the audit seal.
- [ ] No verification tables, seal fingerprints, `PUBLICATION_STATUS` blocks or
      internal checklists: those belong in the full record.
- [ ] Links point to `main` or the tag, not to a local path.
- [ ] `CHANGELOG.md` remains the canonical per-change list; the note links to it.

`v0.9.3.md` here is the body published on 2026-11-28, after the page was reduced
from 612 lines to 27.
