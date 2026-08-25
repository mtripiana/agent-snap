# Security model

## Trust assumptions

The daemon process and its owner are trusted. The command running inside a
snapshot is not trusted only when Linux hardened mode has passed preflight.
The workspace may contain sensitive files, so callers must still choose a
workspace and runtime environment deliberately.

## Enforcement modes

- `best_effort` applies portable process limits and attempts Linux namespace
  isolation. Failures are reported as compatibility behavior and are not a
  security guarantee.
- `hardened` is Linux-only, requires all doctor checks plus `bubblewrap`,
  disallows shell command strings, and requires network isolation. If any
  prerequisite is missing, execution fails before the command starts.

The verified backend passes an explicit cBPF seccomp profile to bubblewrap,
drops capabilities, disables nested user namespaces, applies cgroup v2 and
resource limits, and kills the complete process group on timeout.

The hardened runtime exposes only the prepared workspace and read-only system
runtime paths needed to start the command. It is still not a VM boundary and
must not be treated as protection against a compromised host kernel.

## Out of scope

AgentSnap does not provide multi-user daemon authorization, remote access
control, protection from a hostile daemon owner, or automatic redaction of
secrets in command output and audit logs.
