from __future__ import annotations

import json
import os
import socket
from dataclasses import dataclass
from typing import Any, Dict, Mapping, Optional, Sequence


@dataclass
class ExecutionResult:
    exit_code: int
    stdout: str
    stderr: str
    duration_ms: int
    effective_isolation: str = "best_effort"
    enforcement_report: Optional[Mapping[str, Any]] = None


class AgentSnapError(RuntimeError):
    """Raised when the daemon returns a JSON-RPC error."""


class Sandbox:
    def __init__(
        self,
        workspace: str,
        socket_path: Optional[str] = None,
        read_only: bool = False,
        socket_timeout: float = 30.0,
    ):
        self.workspace = workspace
        self.socket_path = socket_path or os.getenv("AGENTSNAP_SOCKET", "/tmp/agentsnap.sock")
        self.read_only = read_only
        self.socket_timeout = socket_timeout
        self.snapshot_id: Optional[str] = None
        self._request_id = 0

    def __enter__(self) -> "Sandbox":
        self.create_snapshot()
        return self

    def __exit__(self, exc_type, exc_val, exc_tb) -> None:
        if self.snapshot_id is not None:
            self.rollback()
        return False

    def create_snapshot(self) -> str:
        response = self._rpc(
            "agentsnap.create_snapshot",
            {"workspace": self.workspace, "read_only": self.read_only},
        )
        snapshot_id = response["result"]["snapshot_id"]
        self.snapshot_id = snapshot_id
        return snapshot_id

    def exec(
        self,
        command: Optional[str] = None,
        timeout_ms: int = 30_000,
        env: Optional[Dict[str, str]] = None,
        *,
        command_path: Optional[str] = None,
        args: Optional[Sequence[str]] = None,
        policy: Optional[Mapping[str, Any]] = None,
        max_output_bytes: int = 4 * 1024 * 1024,
    ) -> ExecutionResult:
        if self.snapshot_id is None:
            self.create_snapshot()

        if (command is None) == (command_path is None):
            raise ValueError("provide exactly one of command or command_path")

        params: Dict[str, Any] = {
            "snapshot_id": self.snapshot_id,
            "timeout_ms": timeout_ms,
            "env": dict(env or {}),
            "max_output_bytes": max_output_bytes,
        }
        if command is not None:
            params["command"] = command
        else:
            params["command_path"] = command_path
            params["args"] = list(args or [])
        if policy is not None:
            params["policy"] = dict(policy)

        response = self._rpc(
            "agentsnap.execute",
            params,
        )
        payload = response["result"]
        return ExecutionResult(
            exit_code=int(payload["exit_code"]),
            stdout=str(payload.get("stdout", "")),
            stderr=str(payload.get("stderr", "")),
            duration_ms=int(payload.get("duration_ms", 0)),
            effective_isolation=str(payload.get("effective_isolation", "best_effort")),
            enforcement_report=payload.get("enforcement_report"),
        )

    def rollback(self) -> None:
        if self.snapshot_id is None:
            return

        snapshot_id = self.snapshot_id
        self._rpc("agentsnap.rollback", {"snapshot_id": snapshot_id})
        self.snapshot_id = None

    def _rpc(self, method: str, params: Dict) -> Dict:
        self._request_id += 1
        payload = json.dumps(
            {"jsonrpc": "2.0", "id": self._request_id, "method": method, "params": params}
        ).encode("utf-8")

        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as conn:
            conn.settimeout(self.socket_timeout)
            conn.connect(self.socket_path)
            conn.sendall(payload)
            conn.shutdown(socket.SHUT_WR)

            chunks = []
            while True:
                chunk = conn.recv(65535)
                if not chunk:
                    break
                chunks.append(chunk)

        response = json.loads(b"".join(chunks).decode("utf-8").strip())
        if "error" in response and response["error"] is not None:
            error = response["error"]
            raise AgentSnapError(f"daemon error {error.get('code')}: {error.get('message')}")

        return response
