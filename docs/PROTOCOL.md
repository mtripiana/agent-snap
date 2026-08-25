# JSON-RPC protocol

AgentSnap speaks JSON-RPC 2.0 over a local Unix stream socket. Each request
must be no larger than 1 MiB and each response echoes the request `id`. Clients
should send `jsonrpc: "2.0"`; the daemon always emits that version and rejects
an explicitly supplied unsupported version.

## Create a snapshot

```json
{"jsonrpc":"2.0","id":1,"method":"agentsnap.create_snapshot","params":{"workspace":"/path/project","read_only":false}}
```

The result contains `snapshot_id` and `latency_ms`.

## Execute

Use `command` for shell execution in best-effort mode, or use `command_path`
and `args` for direct execution:

```json
{
  "jsonrpc":"2.0",
  "id":2,
  "method":"agentsnap.execute",
  "params":{
    "snapshot_id":"snap_...",
    "command_path":"/bin/echo",
    "args":["hello"],
    "timeout_ms":30000,
    "max_output_bytes":4194304,
    "policy":{
      "isolation_mode":"best_effort",
      "network_mode":"restricted",
      "shell_mode":"disallow",
      "resource_limits":{
        "memory_limit_bytes":536870912,
        "cpu_quota_percent":100,
        "pids_max":128
      },
      "seccomp_profile":"default_script"
    }
  }
}
```

The result contains exit status, captured output, duration, effective mode,
and an enforcement report. Output is capped and the report notes truncation.

## Rollback and errors

Rollback accepts `snapshot_id` and returns `status` and `duration_ms`. Errors
use JSON-RPC error objects with stable negative codes and human-readable
messages. A missing snapshot is `-32001`; execution failure is `-32002`; and
rollback failure is `-32003`.
