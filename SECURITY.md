# Security Policy

## Supported versions

Only the latest released version receives security fixes. Development builds
are experimental and must not be used as a production security boundary.

## Reporting a vulnerability

Please use GitHub Private Vulnerability Reporting when enabled. If it is not
available, contact the maintainers privately through the address listed in the
repository security settings. Include the affected version, platform, exact
reproduction steps, impact, and any safe mitigation.

Do not disclose an unpatched vulnerability in a public issue or pull request.

## Security model

Hardened execution is Linux-first and only claims enforcement when the daemon
preflight checks pass. macOS and other compatibility paths are not a security
boundary. Do not expose the daemon socket to untrusted users or run it on a
machine containing credentials that the workload should not be able to read.
