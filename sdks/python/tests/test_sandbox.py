from agentsnap import Sandbox


class FakeSandbox(Sandbox):
    def __init__(self, workspace: str):
        super().__init__(workspace=workspace, socket_path="/tmp/unused.sock")
        self.calls = []

    def _rpc(self, method, params):
        self.calls.append((method, params))
        if method == "agentsnap.create_snapshot":
            return {"result": {"snapshot_id": "snap_test"}}
        if method == "agentsnap.execute":
            return {
                "result": {
                    "exit_code": 0,
                    "stdout": "ok",
                    "stderr": "",
                    "duration_ms": 2,
                }
            }
        if method == "agentsnap.rollback":
            return {"result": {"status": "restored"}}
        raise AssertionError(f"unexpected method {method}")


def test_context_manager_exec_and_rollback():
    with FakeSandbox(workspace=".") as sandbox:
        result = sandbox.exec("echo ok")
        assert result.exit_code == 0
        assert result.stdout == "ok"

    methods = [name for name, _ in sandbox.calls]
    assert methods == [
        "agentsnap.create_snapshot",
        "agentsnap.execute",
        "agentsnap.rollback",
    ]


def test_direct_command_and_policy_are_forwarded():
    sandbox = FakeSandbox(workspace=".")
    result = sandbox.exec(
        command_path="/bin/echo",
        args=["ok"],
        policy={"isolation_mode": "best_effort"},
    )

    assert result.exit_code == 0
    method, params = sandbox.calls[-1]
    assert method == "agentsnap.execute"
    assert params["command_path"] == "/bin/echo"
    assert params["args"] == ["ok"]
    assert params["policy"]["isolation_mode"] == "best_effort"
