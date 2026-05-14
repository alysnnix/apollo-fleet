# Security Policy

## Supported Versions

Only the latest released version of Apollo Fleet receives security fixes. Older versions are not patched. See [Releases](https://github.com/alysnnix/apollo-fleet/releases) for the current version.

## Reporting a Vulnerability

**Do not open public GitHub issues for security problems.** They are immediately visible to anyone watching the repo.

Instead, please use one of these channels:

1. **GitHub Private Security Advisory** (preferred) — go to the [Security tab](https://github.com/alysnnix/apollo-fleet/security/advisories/new) and submit a private report. Only the repository maintainers receive it.
2. **Email** — write to `aly@alysnnix.dev` with the subject prefix `[apollo-fleet security]`. Encrypt with my public key if you handle sensitive material; the key is on my GitHub profile.

### What to include

- A clear description of the issue and its impact
- Steps to reproduce (config snippets, command lines, version of Apollo Fleet, Windows build, Apollo/Sunshine version)
- Any proof-of-concept code or pcap, if available
- Whether the issue is already public or coordinated with another party

### Response timeline

- **Acknowledgement**: within 72 hours of report
- **Triage and severity assessment**: within 7 days
- **Fix and release**: depends on severity. Critical issues (RCE, privilege escalation, persistent compromise of the host) are prioritized and typically released within 14 days. Lower-severity issues may take longer.

### Coordinated disclosure

We prefer coordinated disclosure. Please give us a reasonable window to fix and release before going public — 30 days is the default. If active exploitation is observed, we may release a fix faster and disclose sooner.

## Scope

In scope:
- The Apollo Fleet binary and source in this repository
- Build / release pipeline (`.github/workflows/`)
- Configuration handling (`seats.toml`, generated `sunshine.conf`, `apps.json`, `fleet-state.json`)
- The `--launch` callback flow used by Apollo's `apps.json`

Out of scope:
- Vulnerabilities in Apollo / Sunshine itself — please report those upstream at the [Apollo](https://github.com/ClassicOldSong/Apollo) and [Sunshine](https://github.com/LizardByte/Sunshine) repositories
- Vulnerabilities in Moonlight clients
- Issues that require pre-existing administrator access on the host machine (Apollo Fleet runs with the privileges granted to the user; an attacker with admin can already replace the binary)
- Social engineering, phishing, or physical access

## Hall of Fame

Reporters who responsibly disclose security issues will be credited in the release notes (with consent) and in this section.

_None yet._
