# AgentSnap Python SDK

Python SDK for communicating with the AgentSnap daemon.

## Install (local editable)

From repository root:

```bash
python3 -m venv .venv
.venv/bin/python -m pip install -e sdks/python
```

## Usage

```python
from agentsnap import Sandbox

with Sandbox(workspace=".") as snap:
    result = snap.exec("echo hello")
    print(result.exit_code)
    print(result.stdout)
```

Direct execution and policy options are also supported:

```python
result = snap.exec(
    command_path="/bin/echo",
    args=["hello"],
    timeout_ms=10_000,
    policy={"isolation_mode": "best_effort", "shell_mode": "disallow"},
)
```

Socket operations time out after 30 seconds by default. Set `socket_timeout`
to change it. Output is capped at 4 MiB by default.

## Configuration

- Default socket: `/tmp/agentsnap.sock`
- Override with constructor argument:

```python
Sandbox(workspace=".", socket_path="/tmp/agentsnap-dev.sock")
```

or environment variable:

```bash
export AGENTSNAP_SOCKET=/tmp/agentsnap-dev.sock
```

## Run tests

```bash
cd sdks/python
python3 -m pytest -q
```
