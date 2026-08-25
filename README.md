# AgentSnap

Experiment faster and safer: run AI-driven code changes in disposable
snapshots with instant rollback.

**Status:** Experimental. Hardened mode is Linux-only; run untrusted workloads
in disposable environments.

AgentSnap provides local workspace snapshots and controlled command execution
for AI coding agents and CLI workflows.

> **Security status:** AgentSnap is experimental. Hardened execution is
> Linux-first and fail-closed. macOS and other portable snapshot paths are
> compatibility modes, not security boundaries. Do not run untrusted code
> with compatibility mode or on a host containing credentials you need to
> protect.

## 2-minute quickstart

```bash
# 1) start daemon (terminal A)
cargo run -p agentsnap-daemon -- --socket /tmp/agentsnap.sock

# 2) check capabilities (terminal B)
cargo run -p agentsnap-cli -- doctor --local-only

# 3) run one disposable command (terminal B)
cargo run -p agentsnap-cli -- exec --workspace . --command "echo hello"
```

If step 3 succeeds, AgentSnap is working in best-effort mode on your machine.

## Known limitations

- Hardened mode is Linux-only and requires `bubblewrap` plus passing doctor checks.
- Best-effort mode is compatibility-oriented and is not a security boundary.
- AgentSnap uses a local Unix socket; do not expose it over network interfaces.
- This project is experimental and should be validated in disposable environments first.

## What is implemented

- Rust snapshot core with Linux OverlayFS and a portable copy-tree fallback.
- Unix-socket daemon with versioned JSON-RPC requests.
- CLI, Python SDK, and Node SDK clients.
- Best-effort process limits on supported Unix platforms.
- Linux hardened mode only when the preflight checks and `bubblewrap` runtime
  are available.

Hardened mode is intended for isolated Linux runners. It is not a replacement
for a VM, container host boundary, or a complete secret-management strategy.
Review `SECURITY.md` before using it with untrusted workloads.

## Platform support

| Platform | Snapshots | Best-effort execution | Hardened execution |
| --- | --- | --- | --- |
| Ubuntu 24.04 x86_64 | Supported | Supported | Supported only after `agentsnap doctor` passes |
| macOS | Compatibility | Best effort only | Unsupported |
| Other platforms | Untested | Unsupported | Unsupported |

## Which mode should I use?

| Goal | Recommended mode | How to run |
| --- | --- | --- |
| Fast local automation on trusted code | `best_effort` | `agentsnap exec --workspace . --command "..."` |
| Running untrusted code with stronger controls (Linux) | `hardened` | `agentsnap exec --workspace . --command-path ... --isolation hardened --network none --no-shell` |
| macOS experimentation | `best_effort` | same as fast local automation |

## Build and test

Rust 1.81, Python 3.9+, Node.js 20+, and (for the cloud examples) Terraform
1.5+ are supported.

```bash
cargo test --workspace --all-targets
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
(cd sdks/python && python -m pytest -q)
(cd sdks/node && npm test)
```

The latency benchmark is a report, not a portability guarantee:

```bash
cargo bench -p agentsnap-core --bench snapshot_latency
```

## Daemon and CLI

Start the daemon:

```bash
cargo run -p agentsnap-daemon -- --socket /tmp/agentsnap.sock
```

Run a command in a disposable snapshot:

```bash
cargo run -p agentsnap-cli -- exec --workspace . --command "echo hello"
```

Check host capabilities:

```bash
cargo run -p agentsnap-cli -- doctor --local-only
```

Hardened execution requires a command path, no shell string, and no network:

```bash
cargo run -p agentsnap-cli -- exec \
  --workspace . \
  --command-path /bin/echo --arg hello \
  --isolation hardened --network none --no-shell
```

On Ubuntu, install the runtime with `sudo apt-get install bubblewrap`. The
daemon reports hardened mode as unavailable when `bwrap`, user namespaces,
seccomp, or cgroup v2 are not usable.

## Linux hardened verification before launch

Before you announce public hardened support, validate from an isolated Linux VM
or runner (not your daily host):

1. Run `cargo run -p agentsnap-cli -- doctor --local-only` and confirm
   `hardened_supported` is `true`.
2. Run `cargo test -p agentsnap-core --test linux_security -- --nocapture`.
3. Run a hardened CLI execution using `--command-path ... --isolation hardened
   --network none --no-shell`.

For disposable cloud validation, use the Terraform bundles under
[`infra/terraform`](infra/terraform) and the shortcuts in `Makefile`.

The daemon socket is intended for the local user and is created with private
permissions. Never expose it over a network socket or mount it into an
untrusted environment.

## SDKs

Python:

```python
from agentsnap import Sandbox

with Sandbox(".") as sandbox:
    result = sandbox.exec("pytest -q")
    print(result.exit_code, result.stdout)
```

Node:

```js
import { Sandbox } from "@agentsnap/sdk";

const sandbox = await Sandbox.create({ workspace: "." });
try {
  const result = await sandbox.exec("echo hello");
  console.log(result.exitCode, result.stdout);
} finally {
  await sandbox.rollback();
}
```

See [`sdks/python/README.md`](sdks/python/README.md) and
[`sdks/node/README.md`](sdks/node/README.md) for installation and advanced
options.

## JSON-RPC

Requests use JSON-RPC 2.0 over the Unix socket. Supported methods are:

- `agentsnap.create_snapshot`
- `agentsnap.execute`
- `agentsnap.rollback`
- `agentsnap.doctor`

`agentsnap.execute` accepts either `command` for an allowed shell command or
`command_path` plus `args` for direct execution. Policies include isolation,
network, shell, seccomp profile, resource limits, timeout, and output limits.
The daemon returns an echoed request ID, an `effective_isolation` value, and an
`enforcement_report`.

## Cloud demos

The AWS and GCP Terraform bundles under [`infra/terraform`](infra/terraform)
are experimental disposable examples. Use a throwaway project, restrict SSH
to your own `/32` CIDR, never provide production credentials, and always run
`terraform destroy` afterward.

## Contributing and security

See [`CONTRIBUTING.md`](CONTRIBUTING.md), [`SECURITY.md`](SECURITY.md), and
[`SUPPORT.md`](SUPPORT.md). The post-upload GitHub settings checklist is in
[`docs/GITHUB_SETTINGS.md`](docs/GITHUB_SETTINGS.md). Please report suspected
vulnerabilities privately.

## Release process

- `Release Please` runs on every push to `main` and keeps a release PR updated
  from conventional commits and `CHANGELOG.md` policy.
- Merging the release PR creates a GitHub release/tag.
- The `Release` workflow publishes artifacts/packages when that release is
  published, and can also be run manually with `workflow_dispatch` plus
  `release_tag`.
