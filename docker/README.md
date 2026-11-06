# vislox in Docker

The image `ghcr.io/hakubo/vislox` holds vislox, git and the [lox](https://github.com/discostu105/lox) CLI.
It is built for `linux/amd64` and `linux/arm64`.

Tags:

- `main`: the latest build of `main`.
- `sha-<commit>`: one particular build.
- `X.Y.Z`, `X.Y` and `latest`: releases, from tags `vX.Y.Z`.

The container serves the UI on port 7878. It keeps the config repository in the volume `/data`.

## Run

Clone a config repository from git and connect to the Miniserver:

```bash
docker run -d --name vislox -p 7878:7878 -v vislox-data:/data \
  -e VISLOX_GIT_URL=https://user:token@git.example.com/me/loxone-config.git \
  -e VISLOX_PATH=/data/repo/project \
  -e LOX_HOST=https://192.168.1.10 -e LOX_USER=admin -e LOX_PASS=secret \
  ghcr.io/hakubo/vislox:main
```

Or view a directory or file you already have: mount it and point `VISLOX_PATH` at it.
For example, `-v ~/loxone-config:/data/repo -e VISLOX_PATH=/data/repo`.

With Docker Compose or a Portainer stack, use [`compose.yml`](compose.yml) and set the variables below.

## Settings

| Variable | |
|---|---|
| `VISLOX_GIT_URL` | Config repository to clone into `/data/repo` on first start. Put credentials in the URL if Apply and Sync should push. |
| `VISLOX_GIT_SSH_KEY` | Private SSH key for the repository (e.g. a Gitea/GitHub deploy key with write access), as is or base64-encoded. Hosts are trusted on first use. |
| `VISLOX_GIT_BRANCH` | Branch to clone (default: the remote's default). |
| `VISLOX_GIT_PULL_SECONDS` | Fast-forward `/data/repo` from its remote this often, so commits pushed elsewhere show up in History. Skipped while there are uncommitted changes. `0` turns it off. Compose default: `60`. |
| `VISLOX_PATH` | What to view: a `lox.toml` project, a `lox config init` repository, a `.Loxone` file or a backup `.zip`. Default: `/data/repo` if it exists, else `/data`. |
| `LOX_HOST`, `LOX_USER`, `LOX_PASS`, `LOX_SERIAL`, `LOX_VERIFY_SSL` | The Miniserver, written to lox's `~/.lox/config.yaml`. Needed for live values, Pull, Sync and Apply. |
| `VISLOX_ADMIN_TOKEN` | Turns on Apply, Discard and Sync, which ask for this token. |
| `GIT_AUTHOR_NAME`, `GIT_AUTHOR_EMAIL` | Who commits for Apply and Sync. |
| `VISLOX_ARGS` | More `vislox` arguments, e.g. `--no-pull` or `--serial 504F94AABBCC`. |
| `VISLOX_PORT` | Host port (compose only, default `7878`). |
| `VISLOX_MONITOR` | `1` turns on the Debug-Monitor (default off): vislox listens for the Miniserver's debug output on UDP, and the Log panel can start and stop it. |
| `VISLOX_MONITOR_IP` | The Docker host's LAN address (e.g. `192.168.1.82`): where the Miniserver sends the Debug-Monitor. Needed in Docker, since vislox would otherwise name the container's own address. |
| `VISLOX_MONITOR_PORT` | The UDP port, published by compose as `<port>/udp` (default `7777`, the Miniserver's Debug-Monitor port). |
| `VISLOX_MONITOR_MINUTES` | How long a Start lasts before vislox stops it (default `60`). |

The Debug-Monitor needs the UDP port published (compose does it: `7777:7777/udp`); Docker's default bridge network
gets no UDP in otherwise. vislox keeps only packets from the Miniserver's address. If the Log panel says packets from
another address (like `172.17.0.1`) were dropped, Docker rewrote the sender: run the container with
`network_mode: host` instead. To turn the Debug-Monitor off again, click **Stop** (or let the time limit end it), then
set `VISLOX_MONITOR` back to `0`.

## Portainer, kept up to date

Each build of `main` moves the branch `deploy` to that commit. On that branch, `docker/compose.yml` pins the image
to the new `sha-<commit>` tag, so Portainer redeploys only once the image exists.

To set it up, create a stack from Git:

- repository `https://github.com/hakubo/vislox`
- reference `refs/heads/deploy`
- compose path `docker/compose.yml`
- turn on GitOps updates (polling, e.g. every 5 minutes)

Set the variables above as the stack's environment variables.
