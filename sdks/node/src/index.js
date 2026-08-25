import net from "node:net";

const DEFAULT_SOCKET = process.env.AGENTSNAP_SOCKET || "/tmp/agentsnap.sock";

export class AgentSnapError extends Error {
  constructor(message, code = undefined) {
    super(message);
    this.name = "AgentSnapError";
    this.code = code;
  }
}

export class Sandbox {
  constructor({ workspace, socketPath = DEFAULT_SOCKET, readOnly = false, socketTimeoutMs = 30000 } = {}) {
    this.workspace = workspace;
    this.socketPath = socketPath;
    this.readOnly = readOnly;
    this.socketTimeoutMs = socketTimeoutMs;
    this.snapshotId = null;
    this.requestId = 0;
  }

  static async create(options) {
    const sandbox = new Sandbox(options);
    await sandbox.createSnapshot();
    return sandbox;
  }

  async createSnapshot() {
    const response = await this._rpc("agentsnap.create_snapshot", {
      workspace: this.workspace,
      read_only: this.readOnly,
    });

    this.snapshotId = response.result.snapshot_id;
    return this.snapshotId;
  }

  async exec(command, { commandPath, args = [], timeoutMs = 30000, env = {}, policy, maxOutputBytes = 4 * 1024 * 1024 } = {}) {
    if (!this.snapshotId) {
      await this.createSnapshot();
    }

    if ((command == null) === (commandPath == null)) {
      throw new TypeError("provide exactly one of command or commandPath");
    }

    const params = {
      snapshot_id: this.snapshotId,
      timeout_ms: timeoutMs,
      env,
      max_output_bytes: maxOutputBytes,
    };
    if (command != null) params.command = command;
    else {
      params.command_path = commandPath;
      params.args = args;
    }
    if (policy != null) params.policy = policy;

    const response = await this._rpc("agentsnap.execute", params);

    return {
      exitCode: Number(response.result.exit_code ?? -1),
      stdout: String(response.result.stdout ?? ""),
      stderr: String(response.result.stderr ?? ""),
      durationMs: Number(response.result.duration_ms ?? 0),
      effectiveIsolation: String(response.result.effective_isolation ?? "best_effort"),
      enforcementReport: response.result.enforcement_report ?? null,
    };
  }

  async rollback() {
    if (!this.snapshotId) {
      return;
    }

    await this._rpc("agentsnap.rollback", { snapshot_id: this.snapshotId });
    this.snapshotId = null;
  }

  async _rpc(method, params) {
    this.requestId += 1;
    const payload = JSON.stringify({ jsonrpc: "2.0", id: this.requestId, method, params });

    return new Promise((resolve, reject) => {
      const client = net.createConnection(this.socketPath);
      const chunks = [];

      client.setTimeout(this.socketTimeoutMs, () => {
        client.destroy(new AgentSnapError(`daemon request timed out after ${this.socketTimeoutMs}ms`));
      });

      client.on("connect", () => {
        client.write(payload);
        client.end();
      });

      client.on("data", (chunk) => {
        chunks.push(chunk);
      });

      client.on("end", () => {
        try {
          const text = Buffer.concat(chunks).toString("utf8").trim();
          const response = JSON.parse(text);
          if (response.error) {
            reject(new AgentSnapError(
              `daemon error ${response.error.code}: ${response.error.message}`,
              response.error.code,
            ));
            return;
          }
          resolve(response);
        } catch (error) {
          reject(error);
        }
      });

      client.on("error", reject);
    });
  }
}
