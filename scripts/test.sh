#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

cargo test --workspace --all-targets

if [ -z "${PYTHON:-}" ] && [ -x "$repo_root/.venv/bin/python" ]; then
  python_bin="$repo_root/.venv/bin/python"
else
  python_bin="${PYTHON:-python3}"
fi
command -v "$python_bin" >/dev/null 2>&1 || [ -x "$python_bin" ] || {
  echo "[error] Python is required; install Python 3.9+ or set PYTHON" >&2
  exit 1
}
if [[ "$python_bin" != /* && -x "$repo_root/$python_bin" ]]; then
  python_bin="$repo_root/$python_bin"
fi
"$python_bin" -c 'import pytest' >/dev/null 2>&1 || {
  echo "[error] pytest is required; install it with: python -m pip install pytest" >&2
  exit 1
}
(cd sdks/python && "$python_bin" -m pytest -q)

command -v npm >/dev/null 2>&1 || {
  echo "[error] npm is required for Node SDK tests" >&2
  exit 1
}
(cd sdks/node && npm test)
