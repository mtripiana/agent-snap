# AgentSnap Node SDK

Node SDK for communicating with the AgentSnap daemon.

## Install

From repository root:

```bash
cd sdks/node
npm install
```

## Usage

```js
import { Sandbox } from "@agentsnap/sdk";

const snap = await Sandbox.create({ workspace: "." });
try {
  const result = await snap.exec("echo hello");
  console.log(result.exitCode, result.stdout);
} finally {
  await snap.rollback();
}
```

Direct execution and policy options are supported as well:

```js
const result = await snap.exec(null, {
  commandPath: "/bin/echo",
  args: ["hello"],
  timeoutMs: 10_000,
  policy: { isolation_mode: "best_effort", shell_mode: "disallow" },
});
```

Socket operations time out after 30 seconds by default. Set
`socketTimeoutMs` when creating the sandbox to change it. Output is capped at
4 MiB by default.

## Configuration

- Default socket: `/tmp/agentsnap.sock`
- Override in code:

```js
const snap = await Sandbox.create({ workspace: ".", socketPath: "/tmp/agentsnap-dev.sock" });
```

or with environment variable:

```bash
export AGENTSNAP_SOCKET=/tmp/agentsnap-dev.sock
```

## Run tests

```bash
cd sdks/node
npm test
```
