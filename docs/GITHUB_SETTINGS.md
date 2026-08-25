# GitHub launch settings

Apply these settings before broad public announcement and keep them reviewed as
the release process evolves:

- Enable the dependency graph, Dependabot alerts, Dependabot security updates,
  secret scanning, and private vulnerability reporting.
- Protect the default branch: require pull requests, require the Rust, SDK,
  package, Terraform, security-audit, CodeQL, Release Please, and Linux
  hardened checks, and
  dismiss stale approvals after new commits.
- Restrict Actions to local and approved actions, require approval for
  fork-originated workflows, and prevent untrusted pull requests from
  receiving write permissions or secrets.
- Configure the release environments (`pypi`, `npm`, and `crates-io`) with
  approval rules and the `PYPI_TOKEN`, `NPM_TOKEN`, and
  `CARGO_REGISTRY_TOKEN` secrets. Keep release tags protected.
- Set the repository homepage, description, topics, and default community
  health files after confirming the final owner/repository URL in package
  metadata and issue templates.

Review these settings whenever the default branch or release process changes.
