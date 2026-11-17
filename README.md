# hhek-untis2ics

[![CI](https://github.com/thefutureisprivate/hhek-untis2ics/actions/workflows/ci.yml/badge.svg)](https://github.com/thefutureisprivate/hhek-untis2ics/actions/workflows/ci.yml)
[![OpenSSF Scorecard](https://api.scorecard.dev/projects/github.com/thefutureisprivate/hhek-untis2ics/badge)](https://scorecard.dev/viewer/?uri=github.com/thefutureisprivate/hhek-untis2ics)
[![OpenSSF Baseline](https://www.bestpractices.dev/projects/16846/baseline)](https://www.bestpractices.dev/en/projects/16846/baseline-1)

A small Rust ICS subscription service for Heinrich-Hertz-BK Bonn. It fetches
your personal WebUntis timetable every 15 minutes, covering the previous 14
days and next 90 days, within the dates the school allows.

The server, school identifier and Europe/Berlin timezone are fixed for HHEK.

- Cancelled periods remain in the feed with `[CANCELLED]`, `STATUS:CANCELLED`
  and transparent availability.
- Exams have `[EXAM]` titles, their type/name/notes, `CATEGORIES:EXAM`, high
  priority and a colour hint. Calendar apps decide whether to use the colour.
- Substitutions include current teachers/rooms, original values and the
  school's lesson, substitution and information texts.
- Each WebUntis period is one event, including each period of a multi-period
  exam. UIDs use the upstream period ID; times and status can change without
  creating another event. Unchanged event revisions remain stable between polls.
- Times are converted from school-local Europe/Berlin time to UTC, including
  daylight saving. Mobile API timestamps carry a misleading literal `Z` suffix.

## Authentication with UCS SSO

Sign into <https://heinrich-hertz-bk.webuntis.com> through UCS normally. In
**Profile → Freigaben → Zugriff über Untis Mobile → Anzeigen**, use the
displayed **Benutzer** and **Schlüssel**. The container uses that existing mobile
key to generate authentication codes; it needs neither the UCS password nor an
exported browser session. Revoking mobile devices in WebUntis revokes this access.

The service uses WebUntis's internal mobile JSON-RPC endpoints
(`getUserData2017` and `getTimetable2017`), which may change upstream.

## Run with rootless Podman

Requires current Podman configured for rootless containers.
The published image is `ghcr.io/thefutureisprivate/hhek-untis2ics:latest`.

GHCR packages are private by default. Anonymous pulls require public package
visibility. For a private package, authenticate with `podman login ghcr.io`
using a classic token with `read:packages` access. See
[GHCR authentication and visibility](https://docs.github.com/en/packages/working-with-a-github-packages-registry/working-with-the-container-registry).

```sh
git clone https://github.com/thefutureisprivate/hhek-untis2ics.git
cd hhek-untis2ics
podman pull ghcr.io/thefutureisprivate/hhek-untis2ics:latest
```

Create a private configuration file:

```sh
install -m600 config.example.json config.json
openssl rand -hex 32
```

Edit `config.json`: set `user` to the displayed WebUntis username, `secret` to
the Untis Mobile key, and `feed_token` to the generated 64-character hexadecimal
value. Store the configuration as a Podman secret, then remove the file:

```sh
podman secret create hhek-untis2ics-config config.json
rm config.json
```

Never commit the real configuration.

Run the service with an immutable filesystem and no Linux capabilities:

```sh
podman run -d --name hhek-untis2ics \
  --read-only --read-only-tmpfs=false \
  --cap-drop=ALL --security-opt=no-new-privileges \
  --pids-limit=64 --memory=128m --cpus=1 \
  --secret=hhek-untis2ics-config,target=/etc/hhek-untis2ics/config.json,uid=65532,gid=65532,mode=0400 \
  -p 127.0.0.1:8080:8080 \
  -p 127.0.0.1:9464:9090 \
  ghcr.io/thefutureisprivate/hhek-untis2ics:latest

podman exec hhek-untis2ics /hhek-untis2ics url
curl --fail http://127.0.0.1:8080/healthz
podman logs hhek-untis2ics
```

Subscribe to the printed URL in your calendar app; importing a downloaded file
once will not receive updates. The URL is a bearer credential: anyone with it
can read the timetable. Only loopback is published by default. For access from
other devices, put the service behind HTTPS and keep the secret path out of
proxy access logs. The server does not log URLs, lesson contents or credentials.
The separate feed token grants calendar access without disclosing the mobile key.

## Hardened rootless Quadlet

[`deploy/hhek-untis2ics.container`](deploy/hhek-untis2ics.container) runs the
service under a rootless user manager. Use a dedicated unprivileged service
account with subordinate UID/GID ranges, cgroup v2, current Podman and systemd.
Pull the published image and create the Podman secret as that account first.
`Pull=never` uses the cached GHCR image, keeping registry authentication and
downloads outside the service's resource limits.

The container runs as UID/GID 65532 with the host user excluded from its user
namespace (`nomap`). It drops all capabilities, enables no-new-privileges, uses
an immutable root filesystem with no writable application tmpfs or data mounts,
and retains Podman's default seccomp and SELinux/AppArmor confinement. Private
pasta networking disables gateway mapping; both published ports bind only to
127.0.0.1. DNS and HTTPS access are required for WebUntis; the network namespace
does not implement a destination firewall.

The Quadlet limits the container to 128 MiB, one CPU and 64 processes, with
core dumps disabled and 1024 file descriptors. The service cgroup additionally
caps aggregate container/launcher memory at 192 MiB, disables swap, and limits
the whole service to one CPU and 96 tasks. The existing secret is mounted
read-only with mode 0400. The Quadlet adds no host directory, device, or Podman
socket mounts. Restart failures are rate-limited.

Stop a manually started container with the same name before switching to the
Quadlet. Install the unit as the service account:

```sh
install -Dm600 deploy/hhek-untis2ics.container \
  "$HOME/.config/containers/systemd/hhek-untis2ics.container"
systemctl --user daemon-reload
systemctl --user start hhek-untis2ics.service
systemctl --user status hhek-untis2ics.service
journalctl --user -u hhek-untis2ics.service
```

`WantedBy=default.target` starts the generated unit with the user manager;
Quadlet-generated services are not enabled with `systemctl enable`. For startup
at boot without login, enable lingering for the service account with
`loginctl enable-linger "$USER"`.

To deploy an updated image:

```sh
podman pull ghcr.io/thefutureisprivate/hhek-untis2ics:latest
systemctl --user restart hhek-untis2ics.service
```

Use an image digest in the Quadlet's `Image=` value to pin a specific build.

Apply container restrictions through the `[Container]` options. Host-level
`NoNewPrivileges`, `RestrictNamespaces` or filesystem restrictions on the Podman
launcher can prevent rootless namespace setup or access to its image store.

## Prometheus monitoring

The monitoring listener serves `GET /metrics` on container port 9090, published
only at `127.0.0.1:9464` to leave Prometheus's own default port free. It uses
Prometheus text exposition, with fixed metric names and numeric values. It has
no calendar route, and `/metrics` is not exposed on the calendar listener at
port 8080.
It returns HTTP 200 even before a calendar is available or after a refresh
failure, so Prometheus can distinguish an available exporter from stale data.
No account identifiers, lesson text, event IDs or feed credentials are exported.

[`deploy/prometheus.yml`](deploy/prometheus.yml) is a scrape example for
Prometheus running on the same host. Merge its job into your Prometheus config:

```sh
curl --fail http://127.0.0.1:9464/metrics
```

| Metric prefix: `hhek_untis2ics_` | Meaning |
| --- | --- |
| `ready` | 1 when a snapshot less than one hour old is available |
| `refresh_successes_total` | Successful refreshes since process start |
| `refresh_failures_total` | Failed upstream fetches or invalid snapshots |
| `last_refresh_success` | Result of the last completed refresh, initially 0 |
| `last_refresh_duration_seconds` | Duration of the last completed refresh |
| `last_success_timestamp_seconds` | Unix time of the last successful refresh, initially 0 |
| `calendar_events` | Event count from the last successful snapshot |

Counters reset on restart. A failed refresh preserves the last successful
timestamp, event count and cached calendar; readiness becomes 0 after one hour
without success. Useful alerts are `up{job="hhek-untis2ics"} == 0` and
`hhek_untis2ics_ready == 0`, with a suitable startup grace period.

Inside a separate Prometheus container, `127.0.0.1` refers to that container.
Use a deliberately configured private monitoring network or host-side agent
for that deployment. Keep the monitoring port private and proxy only the calendar
endpoint if publishing subscriptions through HTTPS.

## Refresh and failure behaviour

`/healthz` reports readiness without revealing calendar data. Until the first
successful refresh, the feed returns HTTP 503. A failed refresh leaves the last
successful snapshot intact for up to one hour; then the calendar feed and
`/healthz` return 503 until a complete refresh succeeds. Invalid timestamps,
malformed responses and duplicate period IDs reject the entire refresh. There
is no persistent timetable database or account data in the image. A restart
refetches the calendar and
advances its revisions. A period removed entirely by WebUntis disappears from
the subscription; periods explicitly marked cancelled remain marked cancelled.

Calendar apps choose their own refresh schedule and may hide events marked
cancelled. The title also carries the cancellation label for apps that display
them. Updates arrive after the service's next poll and the calendar app's next
refresh, rather than as push notifications.

To rotate either credential, replace the Podman secret and recreate the
container. Podman secrets are local secret storage, not an encrypted vault; keep
the host account protected.

## Development

Build the image locally:

```sh
podman build --pull --no-cache -t localhost/hhek-untis2ics .
```

Use `localhost/hhek-untis2ics` in the run command for local development.
The build runs formatting, Clippy with warnings as errors, and tests before
producing a non-root `scratch` image. The musl binary is built with the official
Alpine Rust image, pinned by digest. The runtime contains the binary, CA
certificates, the Europe/Berlin timezone file and license notices; it has no shell
or package manager. `Cargo.lock` locks registry dependencies and checksums.

Native development requires Rust 1.100 or later. There are seven direct runtime
dependencies: Axum/Tokio for HTTP, Reqwest with Rustls for TLS, Serde/serde_json
for JSON, Jiff for dates/timezones, and totp-rs for mobile authentication. Optional
features are restricted; there is no browser automation dependency or WebUntis
wrapper crate.

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

Use `cargo audit` from [cargo-audit](https://github.com/rustsec/rustsec/tree/main/cargo-audit)
to check dependencies against RustSec advisories.

WebUntis's provider-defined 80-bit mobile keys use TOTP with SHA-1, six digits
and a 30-second step. The key-length exception is documented in the code and
covered by RFC test vectors and a provider-key-length test.

For a local binary, `CONFIG_FILE` can point to a private JSON file with the
same format as the example. The calendar listens on port 8080 and metrics on
port 9090. Both bind to all interfaces; container port bindings or a host
firewall control exposure.

## GitHub CI and dependency updates

CI runs on pull requests, pushes to `main`, manual dispatch, and every night at
00:00 UTC (`0 0 * * *`). It builds the Containerfile, which checks formatting,
runs Clippy with warnings as errors, runs the tests, and builds the final
`scratch` image. Builds use `--pull --no-cache` so nightly runs refresh Alpine
packages, CA certificates and timezone data. CI needs no WebUntis credentials.

After the build/tests and lockfile verification pass, a separate job publishes
`ghcr.io/thefutureisprivate/hhek-untis2ics:latest` and `:sha-<full-commit>` on
pushes to `main`, nightly runs, and manual runs targeting `main`. It rebuilds
and tests the same commit before publishing, using the repository's
`GITHUB_TOKEN` with job-scoped `packages: write` permission. Pull requests and
manual runs on other branches only build, test and verify the lockfile.

Both tags refer to the latest build of that commit; nightly builds can refresh
the image without changing the commit. Use a digest for an immutable reference.
The image's source and revision labels link it to the repository and commit.

Dependabot checks the container builder image, GitHub Actions and Rust crates
daily and opens update pull requests. The builder remains digest-pinned.
Actions use exact version tags with verified commit pins and repository
identities in the generated `.github/workflows/actions.lock`. Dependabot updates
the action references and lockfile together. Container, action and crate version
updates use a seven-day cooldown before opening update pull requests, which go
through the same CI checks. Dependabot security updates are exempt from this
cooldown.

A separate CI job runs `gh actions-lock --verify` to verify the
lockfile. When adding or changing action references, regenerate it with an
authenticated GitHub CLI:

```sh
gh extension install github/gh-actions-lock --pin v0.4.1
gh actions-lock
gh actions-lock --verify
```

Review workflow and lockfile changes together; do not edit `actions.lock` by
hand. The verifier extension version is pinned in CI and must be updated there
when upgrading the tool.
The lockfile workflow follows
[Lock your GitHub Actions](https://hugovk.dev/blog/2026/lock-github-actions/).

GitHub documents runtime lockfile enforcement for
[GitHub Enterprise Cloud](https://docs.github.com/en/actions/how-tos/secure-your-work/lock-workflow-dependencies).
The verification job checks lockfile provenance; it does not control which
commits other jobs execute on runners without runtime lockfile enforcement.

OpenSSF Scorecard runs on pushes to `main` and nightly at 00:00 UTC. It publishes
results for the Scorecard badge and uploads SARIF findings to GitHub code
scanning using the workflow's token and OIDC; no personal access token is needed.
Its write permissions are restricted to the analysis job, which does not run on
pull requests or execute project code. Action versions are included in the same
workflow lockfile and Dependabot cooldown. The Baseline badge links to the
project's self-assessment and shows its highest attained level.
The upstream Scorecard action uses a version-tagged container image; the
lockfile pins its action commit but cannot pin that upstream image by digest.

## Contributing

Report bugs and propose changes through GitHub issues and pull requests. Keep
changes small, use maintained dependencies, and run the development checks above.
Major behaviour changes must include automated tests. Never include credentials,
private feed URLs, or personal timetable data in issues, patches, or test fixtures.

For vulnerability reports, see the [security policy](SECURITY.md).

## License

Licensed under [Apache-2.0](LICENSE). The container includes the project license
at `/LICENSE` and third-party notices at `/usr/share/licenses/hhek-untis2ics`.
Dependency notices are collected during each build; see [notice sources](licenses/README.md).

References: [Untis Mobile profile setup](https://help.untis.at/hc/de/articles/360010794740-Wie-lege-ich-mein-Profil-in-Untis-Mobile-an),
[mobile API models and local-time parsing](https://github.com/SapuSeven/BetterUntis/tree/master/api/src/main/kotlin/com/sapuseven/untis/api),
[iCalendar (RFC 5545)](https://www.rfc-editor.org/rfc/rfc5545).
