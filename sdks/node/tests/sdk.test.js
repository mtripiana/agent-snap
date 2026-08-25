import test from "node:test";
import assert from "node:assert/strict";

import { Sandbox } from "../src/index.js";

class FakeSandbox extends Sandbox {
  constructor(options) {
    super(options);
    this.calls = [];
  }

  async _rpc(method, params) {
    this.calls.push({ method, params });
    if (method === "agentsnap.create_snapshot") {
      return { result: { snapshot_id: "snap_test" } };
    }
    if (method === "agentsnap.execute") {
      return { result: { exit_code: 0, stdout: "ok", stderr: "", duration_ms: 1 } };
    }
    if (method === "agentsnap.rollback") {
      return { result: { status: "restored" } };
    }
    throw new Error("unexpected method");
  }
}

test("sandbox create/exec/rollback flow", async () => {
  const sandbox = new FakeSandbox({ workspace: "." });

  const result = await sandbox.exec("echo ok");
  assert.equal(result.exitCode, 0);
  assert.equal(result.stdout, "ok");

  await sandbox.rollback();

  assert.deepEqual(
    sandbox.calls.map((call) => call.method),
    ["agentsnap.create_snapshot", "agentsnap.execute", "agentsnap.rollback"],
  );
});

test("sandbox supports command paths and policies", async () => {
  const sandbox = new FakeSandbox({ workspace: "." });
  const result = await sandbox.exec(null, {
    commandPath: "/bin/echo",
    args: ["ok"],
    policy: { isolation_mode: "best_effort" },
  });

  assert.equal(result.effectiveIsolation, "best_effort");
  assert.deepEqual(sandbox.calls[1].params.args, ["ok"]);
  assert.equal(sandbox.calls[1].params.policy.isolation_mode, "best_effort");
});
