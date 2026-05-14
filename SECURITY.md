# Security Policy

Apollo Fleet is open source. We do not practice security through obscurity: vulnerability reports are public, the source is auditable, and patches land in the open.

## Reporting

Open a **public GitHub issue** using the [Security report template](https://github.com/alysnnix/apollo-fleet/issues/new?template=security.yml). Include reproduction steps, impact, and the version you tested against.

If you believe a report is actively dangerous to current users and want to discuss timing before publishing, you may also open a private [GitHub Security Advisory](https://github.com/alysnnix/apollo-fleet/security/advisories/new). The advisory route is optional, not required.

## Supported versions

Only the latest released version receives security fixes. See [Releases](https://github.com/alysnnix/apollo-fleet/releases).

## Response

- Acknowledgement within 7 days of report
- Triage and severity assessment within 14 days
- Fix and release: depends on severity. Critical issues (RCE on the host, privilege escalation, persistent compromise) are prioritized.

There is no formal SLA — this is a personal project. Reasonable best-effort applies.

## Scope

In scope:
- The Apollo Fleet binary and source in this repository
- Build / release pipeline (`.github/workflows/`)
- Configuration handling (`seats.toml`, generated `sunshine.conf`, `apps.json`, `fleet-state.json`)
- The `--launch` callback flow used by Apollo's `apps.json`
- The auto-updater flow (download path, signature checks, restart sequence)
- Dependency supply chain: pinned versions in `Cargo.lock`, `cargo audit` results

Out of scope (report upstream):
- Vulnerabilities in [Apollo](https://github.com/ClassicOldSong/Apollo) or [Sunshine](https://github.com/LizardByte/Sunshine) themselves
- Vulnerabilities in Moonlight clients
- Issues that require pre-existing administrator access on the host (an attacker with admin can already replace the binary directly)
- Social engineering, phishing, or physical access
