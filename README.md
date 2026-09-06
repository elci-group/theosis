# Theosis

Theosis reconciles an installed application's strict `A.B.C` version with the
version declared by its Git remote, then delegates build and installation to
`baby`. It never downgrades automatically and verifies the installed binary
after an update.

## Usage

```sh
theosis check --remote git@github.com:example/app.git --binary ~/.local/bin/app
theosis update --remote git@github.com:example/app.git --ref main \
  --binary ~/.local/bin/app --install-dir ~/.local/bin
```

The remote is cloned into `.theosis/transactions/<id>/` (or the directory
named by `THEOSIS_STATE`) and pinned to the resolved commit. `--json` emits a
stable decision record. Exit code `10` means the local version is newer.

The MVP reads remote versions from Rust `Cargo.toml` `[package] version` and
uses `baby --user` for installation, allowing Baby's normal coordination and
locking policy to apply. Credentials remain under Git's
normal host configuration; Theosis does not store them.
