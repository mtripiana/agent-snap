# Contributing to AgentSnap

AgentSnap is a security-sensitive, Linux-first project. Please read the
security model before changing isolation, filesystem, process, or protocol
behavior.

## Development

Install Rust 1.81, Python 3.9 or newer, Node.js 20 or newer, and Terraform if
you are changing the cloud examples. Run:

```bash
cargo test --workspace --all-targets
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
(cd sdks/python && python -m pytest -q)
(cd sdks/node && npm test)
```

Linux security integration tests require a supported Ubuntu runner and the
`bubblewrap` executable. Do not run untrusted workloads against a development
machine containing credentials.

## Pull requests

Keep changes focused, add regression tests, describe platform assumptions,
and document any public protocol or SDK changes. Security vulnerabilities
must be reported privately using `SECURITY.md`, not through a public issue.
