# OBSOLETE: historical review note

This file recorded an earlier review state that is not the current nginx test
entry behavior.

Current nginx entrypoints are documented in `apps/starry/nginx/README.md` and
`www/nginx-ci-refactor-proposal.md`:

- default/CI smoke uses root `apps/starry/nginx/qemu-<arch>.toml` and runs
  `/usr/bin/nginx-runner.sh smoke`;
- manual all/phase/debug configs live under `apps/starry/nginx/qemu/{all,phase,debug}/`;
- nginx commands use `cargo xtask starry app qemu ...`, not `cargo xtask starry app run ...`.
